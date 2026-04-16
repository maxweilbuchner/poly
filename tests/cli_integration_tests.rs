use poly::client::PolyClient;
use poly::error::AppError;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

// ── helpers ───────────────────────────────────────────────────────────────────

fn fixture(name: &str) -> String {
    std::fs::read_to_string(format!("tests/fixtures/{}", name))
        .unwrap_or_else(|_| panic!("fixture {} not found", name))
}

fn client(server: &MockServer) -> PolyClient {
    PolyClient::new_test(&server.uri(), &server.uri(), &server.uri())
}

// ── search_markets ──────────────────────────────────────────────────────────

#[tokio::test]
async fn search_filters_by_query_in_question() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/markets"))
        .respond_with(ResponseTemplate::new(200).set_body_string(fixture("search_results.json")))
        .mount(&server)
        .await;

    let c = client(&server);
    let results = c.search_markets("bitcoin", true, 10).await.unwrap();

    // "bitcoin" appears in questions 1 and 2 (case-insensitive), but not in #3 (S&P 500)
    assert_eq!(results.len(), 2);
    assert!(results[0]
        .question
        .to_lowercase()
        .contains("bitcoin"));
    assert!(results[1]
        .question
        .to_lowercase()
        .contains("bitcoin"));
}

#[tokio::test]
async fn search_respects_limit() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/markets"))
        .respond_with(ResponseTemplate::new(200).set_body_string(fixture("search_results.json")))
        .mount(&server)
        .await;

    let c = client(&server);
    let results = c.search_markets("bitcoin", true, 1).await.unwrap();

    assert_eq!(results.len(), 1);
}

#[tokio::test]
async fn search_returns_empty_on_no_match() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/markets"))
        .respond_with(ResponseTemplate::new(200).set_body_string(fixture("search_results.json")))
        .mount(&server)
        .await;

    let c = client(&server);
    let results = c
        .search_markets("zzz_nonexistent_zzz", true, 10)
        .await
        .unwrap();

    assert!(results.is_empty());
}

#[tokio::test]
async fn search_api_error_propagates() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/markets"))
        .respond_with(
            ResponseTemplate::new(500).set_body_string(r#"{"error":"Internal Server Error"}"#),
        )
        .expect(4) // send_with_retry retries up to 4 times on 5xx
        .mount(&server)
        .await;

    let c = client(&server);
    let err = c.search_markets("test", true, 10).await.unwrap_err();

    match err {
        AppError::Api { status, .. } => assert_eq!(status, 500),
        other => panic!("expected Api error, got {:?}", other),
    }
}

// ── get_market_by_id ────────────────────────────────────────────────────────

#[tokio::test]
async fn get_market_by_id_returns_market() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/markets/0xaaa111"))
        .respond_with(ResponseTemplate::new(200).set_body_string(fixture("single_market.json")))
        .mount(&server)
        .await;

    let c = client(&server);
    let market = c.get_market_by_id("0xaaa111").await.unwrap();

    assert!(market.is_some());
    let m = market.unwrap();
    assert_eq!(m.condition_id, "0xaaa111");
    assert_eq!(m.question, "Will Bitcoin reach $100k?");
    assert_eq!(m.outcomes.len(), 2);
    assert!((m.outcomes[0].price - 0.45).abs() < 0.01);
}

#[tokio::test]
async fn get_market_by_id_returns_none_on_404() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/markets/0xnotfound"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;

    let c = client(&server);
    let result = c.get_market_by_id("0xnotfound").await.unwrap();

    assert!(result.is_none());
}

// ── get_market_by_slug ──────────────────────────────────────────────────────

#[tokio::test]
async fn get_market_by_slug_via_event_endpoint() {
    let server = MockServer::start().await;

    // Event endpoint returns markets
    Mock::given(method("GET"))
        .and(path("/events"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(fixture("event_with_markets.json")),
        )
        .mount(&server)
        .await;

    let c = client(&server);
    let markets = c.get_market_by_slug("championship-finals").await.unwrap();

    assert_eq!(markets.len(), 2);
    assert_eq!(markets[0].condition_id, "0xevent01");
    assert_eq!(markets[1].condition_id, "0xevent02");
}

#[tokio::test]
async fn get_market_by_slug_falls_back_to_markets_endpoint() {
    let server = MockServer::start().await;

    // Event endpoint returns empty
    Mock::given(method("GET"))
        .and(path("/events"))
        .respond_with(ResponseTemplate::new(200).set_body_string("[]"))
        .mount(&server)
        .await;

    // Market endpoint returns a result
    Mock::given(method("GET"))
        .and(path("/markets"))
        .and(query_param("slug", "will-bitcoin-reach-100k"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(format!("[{}]", fixture("single_market.json"))),
        )
        .mount(&server)
        .await;

    let c = client(&server);
    let markets = c
        .get_market_by_slug("will-bitcoin-reach-100k")
        .await
        .unwrap();

    assert_eq!(markets.len(), 1);
    assert_eq!(markets[0].condition_id, "0xaaa111");
}

#[tokio::test]
async fn get_market_by_slug_returns_empty_when_not_found() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/events"))
        .respond_with(ResponseTemplate::new(200).set_body_string("[]"))
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/markets"))
        .respond_with(ResponseTemplate::new(200).set_body_string("[]"))
        .mount(&server)
        .await;

    let c = client(&server);
    let markets = c.get_market_by_slug("nonexistent-slug").await.unwrap();

    assert!(markets.is_empty());
}

// ── get_top_markets ─────────────────────────────────────────────────────────

#[tokio::test]
async fn get_top_markets_returns_by_volume() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/markets"))
        .respond_with(ResponseTemplate::new(200).set_body_string(fixture("search_results.json")))
        .mount(&server)
        .await;

    let c = client(&server);
    let markets = c.get_top_markets(10, None).await.unwrap();

    assert_eq!(markets.len(), 3);
}

#[tokio::test]
async fn get_top_markets_filters_by_category() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/markets"))
        .respond_with(ResponseTemplate::new(200).set_body_string(fixture("search_results.json")))
        .mount(&server)
        .await;

    let c = client(&server);
    let markets = c.get_top_markets(10, Some("finance")).await.unwrap();

    // Only the S&P 500 market has category "finance"
    assert_eq!(markets.len(), 1);
    assert_eq!(markets[0].condition_id, "0xccc333");
}

#[tokio::test]
async fn get_top_markets_respects_limit() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/markets"))
        .respond_with(ResponseTemplate::new(200).set_body_string(fixture("search_results.json")))
        .mount(&server)
        .await;

    let c = client(&server);
    let markets = c.get_top_markets(2, None).await.unwrap();

    assert_eq!(markets.len(), 2);
}

// ── get_order_book edge cases ───────────────────────────────────────────────

#[tokio::test]
async fn order_book_sorts_bids_desc_asks_asc() {
    let server = MockServer::start().await;

    // Scrambled order
    let body = r#"{
        "bids": [
            {"price": "0.50", "size": "10.0"},
            {"price": "0.70", "size": "20.0"},
            {"price": "0.60", "size": "15.0"}
        ],
        "asks": [
            {"price": "0.90", "size": "10.0"},
            {"price": "0.75", "size": "20.0"},
            {"price": "0.80", "size": "15.0"}
        ]
    }"#;

    Mock::given(method("GET"))
        .and(path("/book"))
        .respond_with(ResponseTemplate::new(200).set_body_string(body))
        .mount(&server)
        .await;

    let c = client(&server);
    let book = c.get_order_book("123").await.unwrap();

    // Bids: highest first
    assert!((book.bids[0].price - 0.70).abs() < 0.001);
    assert!((book.bids[1].price - 0.60).abs() < 0.001);
    assert!((book.bids[2].price - 0.50).abs() < 0.001);

    // Asks: lowest first
    assert!((book.asks[0].price - 0.75).abs() < 0.001);
    assert!((book.asks[1].price - 0.80).abs() < 0.001);
    assert!((book.asks[2].price - 0.90).abs() < 0.001);
}

#[tokio::test]
async fn order_book_handles_unparseable_entries_gracefully() {
    let server = MockServer::start().await;

    // One valid, one with garbage price
    let body = r#"{
        "bids": [
            {"price": "0.50", "size": "10.0"},
            {"price": "not_a_number", "size": "20.0"}
        ],
        "asks": [
            {"price": "0.80", "size": "abc"},
            {"price": "0.90", "size": "10.0"}
        ]
    }"#;

    Mock::given(method("GET"))
        .and(path("/book"))
        .respond_with(ResponseTemplate::new(200).set_body_string(body))
        .mount(&server)
        .await;

    let c = client(&server);
    let book = c.get_order_book("123").await.unwrap();

    // Bad entries filtered out
    assert_eq!(book.bids.len(), 1);
    assert_eq!(book.asks.len(), 1);
}

// ── market outcome parsing ──────────────────────────────────────────────────

#[tokio::test]
async fn market_with_missing_outcomes_still_parses() {
    let server = MockServer::start().await;

    let body = r#"{
        "conditionId": "0xminimal",
        "question": "Minimal market",
        "marketSlug": "minimal",
        "slug": "minimal",
        "active": true,
        "closed": false
    }"#;

    Mock::given(method("GET"))
        .and(path("/markets/0xminimal"))
        .respond_with(ResponseTemplate::new(200).set_body_string(body))
        .mount(&server)
        .await;

    let c = client(&server);
    let market = c.get_market_by_id("0xminimal").await.unwrap();

    // Should still parse, just with empty outcomes
    assert!(market.is_some());
    let m = market.unwrap();
    assert_eq!(m.condition_id, "0xminimal");
    assert!(m.outcomes.is_empty());
}

// ── authenticated endpoints without credentials ─────────────────────────────

#[tokio::test]
async fn get_open_orders_without_credentials_returns_auth_error() {
    let server = MockServer::start().await;
    let c = client(&server); // no wallet or auth configured

    let err = c.get_open_orders().await.unwrap_err();
    assert!(err.is_auth());
}

#[tokio::test]
async fn get_order_history_without_credentials_returns_auth_error() {
    let server = MockServer::start().await;
    let c = client(&server);

    let err = c.get_order_history(10).await.unwrap_err();
    assert!(err.is_auth());
}

#[tokio::test]
async fn get_positions_without_credentials_returns_auth_error() {
    let server = MockServer::start().await;
    let c = client(&server);

    let err = c.get_positions().await.unwrap_err();
    assert!(err.is_auth());
}

#[tokio::test]
async fn cancel_order_without_credentials_returns_auth_error() {
    let server = MockServer::start().await;
    let c = client(&server);

    let err = c.cancel_order("some-order-id").await.unwrap_err();
    assert!(err.is_auth());
}

#[tokio::test]
async fn cancel_all_without_credentials_returns_auth_error() {
    let server = MockServer::start().await;
    let c = client(&server);

    let err = c.cancel_all_orders().await.unwrap_err();
    assert!(err.is_auth());
}

// ── error message extraction ────────────────────────────────────────────────

#[tokio::test]
async fn api_error_extracts_error_field() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/book"))
        .respond_with(
            ResponseTemplate::new(400).set_body_string(r#"{"error":"Invalid token ID"}"#),
        )
        .mount(&server)
        .await;

    let c = client(&server);
    let err = c.get_order_book("bad").await.unwrap_err();

    match err {
        AppError::Api { status, message } => {
            assert_eq!(status, 400);
            assert_eq!(message, "Invalid token ID");
        }
        other => panic!("expected Api error, got {:?}", other),
    }
}

#[tokio::test]
async fn api_error_extracts_errormsg_field() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/book"))
        .respond_with(
            ResponseTemplate::new(400)
                .set_body_string(r#"{"errorMsg":"Polymarket specific error"}"#),
        )
        .mount(&server)
        .await;

    let c = client(&server);
    let err = c.get_order_book("bad").await.unwrap_err();

    match err {
        AppError::Api { status, message } => {
            assert_eq!(status, 400);
            assert_eq!(message, "Polymarket specific error");
        }
        other => panic!("expected Api error, got {:?}", other),
    }
}

#[tokio::test]
async fn api_error_falls_back_to_raw_body() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/book"))
        .respond_with(ResponseTemplate::new(502).set_body_string("Bad Gateway"))
        .expect(4) // retried on 5xx
        .mount(&server)
        .await;

    let c = client(&server);
    let err = c.get_order_book("123").await.unwrap_err();

    match err {
        AppError::Api { status, message } => {
            assert_eq!(status, 502);
            assert_eq!(message, "Bad Gateway");
        }
        other => panic!("expected Api error, got {:?}", other),
    }
}
