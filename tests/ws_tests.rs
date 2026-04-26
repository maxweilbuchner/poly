//! Integration tests for the TUI WebSocket spawners.
//!
//! These tests exercise the WS code paths against a local in-process WS server
//! (built on `tokio_tungstenite::accept_async`) so we can verify protocol
//! framing, event emission, and HTTP fallback without touching the real
//! Polymarket endpoints.

use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use poly::auth::ClobAuth;
use poly::client::PolyClient;
use poly::tui::tasks::{spawn_ws_order_book_at_url, spawn_ws_user_channel_at_url};
use poly::tui::AppEvent;
use tokio::net::TcpListener;
use tokio::sync::{mpsc, watch};
use tokio_tungstenite::tungstenite::Message;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

// ── helpers ───────────────────────────────────────────────────────────────────

/// Bind an ephemeral local TCP listener and return (listener, ws://… URL).
async fn bind_ws_server() -> (TcpListener, String) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let url = format!("ws://{}/", addr);
    (listener, url)
}

/// Wait for a single AppEvent matching the predicate, with a 5 s timeout.
async fn next_matching<F: Fn(&AppEvent) -> bool>(
    rx: &mut mpsc::UnboundedReceiver<AppEvent>,
    pred: F,
) -> AppEvent {
    let timeout = tokio::time::sleep(Duration::from_secs(5));
    tokio::pin!(timeout);
    loop {
        tokio::select! {
            _ = &mut timeout => panic!("timed out waiting for matching AppEvent"),
            ev = rx.recv() => {
                let ev = ev.expect("channel closed before matching event");
                if pred(&ev) {
                    return ev;
                }
            }
        }
    }
}

fn dummy_client() -> Arc<PolyClient> {
    // Tests that drive only the WS path don't actually use the client.
    Arc::new(PolyClient::new_test(
        "http://127.0.0.1:1",
        "http://127.0.0.1:1",
        "http://127.0.0.1:1",
    ))
}

// ── ws_order_book ────────────────────────────────────────────────────────────

#[tokio::test]
async fn ws_order_book_sends_subscribe_with_token_ids() {
    let (listener, url) = bind_ws_server().await;

    let (tx, _rx) = mpsc::unbounded_channel::<AppEvent>();
    let (_cancel_tx, cancel_rx) = watch::channel(false);

    spawn_ws_order_book_at_url(
        dummy_client(),
        tx,
        vec![
            ("Yes".into(), "111111".into()),
            ("No".into(), "222222".into()),
        ],
        cancel_rx,
        url,
    );

    let (stream, _) = listener.accept().await.unwrap();
    let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();

    let msg = tokio::time::timeout(Duration::from_secs(2), ws.next())
        .await
        .expect("no subscribe within timeout")
        .expect("ws stream ended")
        .expect("ws read error");

    let text = match msg {
        Message::Text(t) => t,
        other => panic!("expected text frame, got {:?}", other),
    };
    let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(parsed["type"], "market");
    let ids = parsed["assets_ids"].as_array().unwrap();
    let id_strs: Vec<&str> = ids.iter().map(|v| v.as_str().unwrap()).collect();
    assert!(id_strs.contains(&"111111"));
    assert!(id_strs.contains(&"222222"));
}

#[tokio::test]
async fn ws_order_book_emits_event_on_book_frame() {
    let (listener, url) = bind_ws_server().await;

    let (tx, mut rx) = mpsc::unbounded_channel::<AppEvent>();
    let (_cancel_tx, cancel_rx) = watch::channel(false);

    spawn_ws_order_book_at_url(
        dummy_client(),
        tx,
        vec![("Yes".into(), "111111".into())],
        cancel_rx,
        url,
    );

    let (stream, _) = listener.accept().await.unwrap();
    let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();

    // Drain the subscribe frame.
    let _ = tokio::time::timeout(Duration::from_secs(2), ws.next()).await;

    // Send a synthetic book snapshot for token 111111.
    let book_frame = serde_json::json!({
        "type": "book",
        "asset_id": "111111",
        "bids": [
            { "price": "0.74", "size": "100.0" },
            { "price": "0.73", "size": "200.0" }
        ],
        "asks": [
            { "price": "0.75", "size": "150.0" },
            { "price": "0.76", "size": "300.0" }
        ]
    })
    .to_string();
    ws.send(Message::Text(book_frame)).await.unwrap();

    let ev = next_matching(&mut rx, |e| matches!(e, AppEvent::OrderBookUpdated(_))).await;
    let books = match ev {
        AppEvent::OrderBookUpdated(b) => b,
        _ => unreachable!(),
    };
    assert_eq!(books.len(), 1);
    let (label, book) = &books[0];
    assert_eq!(label, "Yes");
    assert_eq!(book.token_id, "111111");
    assert_eq!(book.bids.len(), 2);
    assert_eq!(book.asks.len(), 2);
    // Bids sorted descending, asks ascending.
    assert!(book.bids[0].price > book.bids[1].price);
    assert!(book.asks[0].price < book.asks[1].price);
    assert!((book.bids[0].price - 0.74).abs() < 1e-6);
    assert!((book.asks[0].price - 0.75).abs() < 1e-6);
}

#[tokio::test]
async fn ws_order_book_falls_back_to_http_on_connect_failure() {
    // WireMock serves the order-book HTTP endpoint that the fallback loop hits.
    let http = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/book"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            r#"{
                "asset_id":"111111",
                "bids":[{"price":"0.55","size":"50"}],
                "asks":[{"price":"0.57","size":"60"}]
            }"#,
        ))
        .mount(&http)
        .await;

    let client = Arc::new(PolyClient::new_test(&http.uri(), &http.uri(), &http.uri()));

    // Point WS at a port nothing is listening on → connect_async returns
    // ECONNREFUSED immediately on loopback → fallback loop kicks in.
    let dead_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let dead_addr = dead_listener.local_addr().unwrap();
    drop(dead_listener); // close it; OS will refuse immediately
    let dead_url = format!("ws://{}/", dead_addr);

    let (tx, mut rx) = mpsc::unbounded_channel::<AppEvent>();
    let (_cancel_tx, cancel_rx) = watch::channel(false);

    spawn_ws_order_book_at_url(
        client,
        tx,
        vec![("Yes".into(), "111111".into())],
        cancel_rx,
        dead_url,
    );

    // Fallback loop sleeps 10 s between iterations. Wait up to 15 s for the
    // first HTTP poll to land.
    let timeout = tokio::time::sleep(Duration::from_secs(15));
    tokio::pin!(timeout);
    loop {
        tokio::select! {
            _ = &mut timeout => panic!("HTTP fallback never delivered an OrderBookUpdated"),
            ev = rx.recv() => {
                if let Some(AppEvent::OrderBookUpdated(books)) = ev {
                    assert_eq!(books.len(), 1);
                    assert_eq!(books[0].1.token_id, "111111");
                    assert_eq!(books[0].1.bids.len(), 1);
                    assert!((books[0].1.bids[0].price - 0.55).abs() < 1e-6);
                    return;
                }
            }
        }
    }
}

// ── ws_user_channel ──────────────────────────────────────────────────────────

#[tokio::test]
async fn ws_user_channel_sends_auth_then_emits_connected() {
    let (listener, url) = bind_ws_server().await;

    let auth = ClobAuth::new("api-key".into(), "secret".into(), "passphrase".into());

    let (tx, mut rx) = mpsc::unbounded_channel::<AppEvent>();
    let (_cancel_tx, cancel_rx) = watch::channel(false);

    spawn_ws_user_channel_at_url(auth, tx, cancel_rx, url);

    let (stream, _) = listener.accept().await.unwrap();
    let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();

    let msg = tokio::time::timeout(Duration::from_secs(2), ws.next())
        .await
        .expect("no auth within timeout")
        .expect("ws stream ended")
        .expect("ws read error");

    let text = match msg {
        Message::Text(t) => t,
        other => panic!("expected text frame, got {:?}", other),
    };
    let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(parsed["type"], "user");
    assert_eq!(parsed["auth"]["apiKey"], "api-key");
    assert_eq!(parsed["auth"]["secret"], "secret");
    assert_eq!(parsed["auth"]["passphrase"], "passphrase");

    let ev = next_matching(&mut rx, |e| matches!(e, AppEvent::UserWsConnected)).await;
    assert!(matches!(ev, AppEvent::UserWsConnected));

    // Send a synthetic order update and verify it propagates.
    let frame = serde_json::json!({
        "id": "order-abc",
        "status": "MATCHED"
    })
    .to_string();
    ws.send(Message::Text(frame)).await.unwrap();

    let ev = next_matching(&mut rx, |e| matches!(e, AppEvent::UserOrderUpdate(_, _))).await;
    match ev {
        AppEvent::UserOrderUpdate(id, status) => {
            assert_eq!(id, "order-abc");
            assert_eq!(status, "MATCHED");
        }
        _ => unreachable!(),
    }
}

#[tokio::test]
async fn ws_user_channel_emits_disconnected_on_close() {
    let (listener, url) = bind_ws_server().await;

    let auth = ClobAuth::new("k".into(), "s".into(), "p".into());

    let (tx, mut rx) = mpsc::unbounded_channel::<AppEvent>();
    let (_cancel_tx, cancel_rx) = watch::channel(false);

    spawn_ws_user_channel_at_url(auth, tx, cancel_rx, url);

    let (stream, _) = listener.accept().await.unwrap();
    let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();

    // Wait for the auth frame, confirm Connected fires, then drop the stream
    // to force EOF on the client side (the user-WS code doesn't reciprocate
    // a Close frame on its own, so a graceful close would hang).
    let _ = tokio::time::timeout(Duration::from_secs(2), ws.next()).await;
    let _connected = next_matching(&mut rx, |e| matches!(e, AppEvent::UserWsConnected)).await;
    drop(ws);
    let _disconnected = next_matching(&mut rx, |e| matches!(e, AppEvent::UserWsDisconnected)).await;
}
