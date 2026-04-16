use poly::client::PolyClient;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

// ── helpers ───────────────────────────────────────────────────────────────────

fn fixture(name: &str) -> String {
    std::fs::read_to_string(format!("tests/fixtures/{}", name))
        .unwrap_or_else(|_| panic!("fixture {} not found", name))
}

fn client(gamma: &str, clob: &str, data: &str) -> PolyClient {
    PolyClient::new_test(gamma, clob, data)
}

// ── get_markets_page ──────────────────────────────────────────────────────────

#[tokio::test]
async fn test_get_markets_page_parses_response() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/markets"))
        .respond_with(ResponseTemplate::new(200).set_body_string(fixture("markets_page.json")))
        .mount(&server)
        .await;

    let c = client(&server.uri(), &server.uri(), &server.uri());
    let markets = c.get_markets_page(0, 10).await.unwrap();

    assert_eq!(markets.len(), 2);
    assert_eq!(markets[0].condition_id, "0xabc123");
    assert_eq!(markets[0].question, "Will the test pass?");
    assert!((markets[0].volume - 50000.0).abs() < 1.0);
    assert_eq!(markets[0].outcomes.len(), 2);
    assert!((markets[0].outcomes[0].price - 0.75).abs() < 0.01);
}

#[tokio::test]
async fn test_get_markets_page_skips_empty_condition_id() {
    let server = MockServer::start().await;

    // One valid market, one with empty conditionId (should be filtered by gamma_to_market)
    let body = r#"[
        {"conditionId":"0xvalid","question":"Valid","marketSlug":"valid","active":true,"closed":false},
        {"conditionId":"","question":"","marketSlug":"","active":false,"closed":false}
    ]"#;

    Mock::given(method("GET"))
        .and(path("/markets"))
        .respond_with(ResponseTemplate::new(200).set_body_string(body))
        .mount(&server)
        .await;

    let c = client(&server.uri(), &server.uri(), &server.uri());
    let markets = c.get_markets_page(0, 10).await.unwrap();

    // The empty market should be filtered out by gamma_to_market
    assert_eq!(markets.len(), 1);
    assert_eq!(markets[0].condition_id, "0xvalid");
}

// ── get_order_book ────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_get_order_book_parses_bids_asks() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/book"))
        .respond_with(ResponseTemplate::new(200).set_body_string(fixture("order_book.json")))
        .mount(&server)
        .await;

    let c = client(&server.uri(), &server.uri(), &server.uri());
    let book = c.get_order_book("111111").await.unwrap();

    assert_eq!(book.bids.len(), 3);
    assert_eq!(book.asks.len(), 3);

    // Bids sorted highest first
    assert!(book.bids[0].price >= book.bids[1].price);
    assert!((book.bids[0].price - 0.74).abs() < 0.001);

    // Asks sorted lowest first
    assert!(book.asks[0].price <= book.asks[1].price);
    assert!((book.asks[0].price - 0.75).abs() < 0.001);
}

#[tokio::test]
async fn test_get_order_book_empty() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/book"))
        .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"bids":[],"asks":[]}"#))
        .mount(&server)
        .await;

    let c = client(&server.uri(), &server.uri(), &server.uri());
    let book = c.get_order_book("999").await.unwrap();

    assert!(book.bids.is_empty());
    assert!(book.asks.is_empty());
}

// ── error detection ───────────────────────────────────────────────────────────

#[tokio::test]
async fn test_api_401_returns_api_error() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/book"))
        .respond_with(ResponseTemplate::new(401).set_body_string(r#"{"error":"Unauthorized"}"#))
        .mount(&server)
        .await;

    let c = client(&server.uri(), &server.uri(), &server.uri());
    let err = c.get_order_book("111111").await.unwrap_err();

    match err {
        poly::error::AppError::Api { status, message } => {
            assert_eq!(status, 401);
            assert_eq!(message, "Unauthorized");
        }
        other => panic!("expected Api error, got {:?}", other),
    }
}

#[tokio::test]
async fn test_api_403_returns_api_error() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/markets"))
        .respond_with(ResponseTemplate::new(403).set_body_string(r#"{"message":"Forbidden"}"#))
        .mount(&server)
        .await;

    let c = client(&server.uri(), &server.uri(), &server.uri());
    let err = c.get_markets_page(0, 5).await.unwrap_err();

    match err {
        poly::error::AppError::Api { status, message } => {
            assert_eq!(status, 403);
            assert_eq!(message, "Forbidden");
        }
        other => panic!("expected Api error, got {:?}", other),
    }
}

// ── get_price_history ─────────────────────────────────────────────────────────

#[tokio::test]
async fn test_get_price_history_parses_points() {
    let server = MockServer::start().await;

    let body = r#"{"history":[{"t":1700000000,"p":0.60},{"t":1700003600,"p":0.65},{"t":1700007200,"p":0.70}]}"#;

    Mock::given(method("GET"))
        .and(path("/prices-history"))
        .respond_with(ResponseTemplate::new(200).set_body_string(body))
        .mount(&server)
        .await;

    let c = client(&server.uri(), &server.uri(), &server.uri());
    let points = c.get_price_history("0xabc123", "1d", 60).await.unwrap();

    assert_eq!(points.len(), 3);
    assert_eq!(points[0].0, 1700000000);
    assert!((points[0].1 - 0.60).abs() < 0.001);
    assert!((points[2].1 - 0.70).abs() < 0.001);
}

#[tokio::test]
async fn test_get_price_history_empty() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/prices-history"))
        .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"history":[]}"#))
        .mount(&server)
        .await;

    let c = client(&server.uri(), &server.uri(), &server.uri());
    let points = c.get_price_history("0xabc123", "1d", 60).await.unwrap();

    assert!(points.is_empty());
}

// ── fee calculation ───────────────────────────────────────────────────────────

#[test]
fn test_fee_calculation_peaks_at_50_pct() {
    // Fee should be highest at p=0.50
    let fee_50 = PolyClient::calculate_fee(100.0, 0.50, 100);
    let fee_70 = PolyClient::calculate_fee(100.0, 0.70, 100);
    let fee_30 = PolyClient::calculate_fee(100.0, 0.30, 100);

    assert!(fee_50 > fee_70);
    assert!(fee_50 > fee_30);
}

#[test]
fn test_fee_calculation_symmetric() {
    // p and 1-p should give same fee
    let fee_a = PolyClient::calculate_fee(100.0, 0.25, 100);
    let fee_b = PolyClient::calculate_fee(100.0, 0.75, 100);
    assert!((fee_a - fee_b).abs() < 0.000001);
}

#[test]
fn test_fee_calculation_zero_size() {
    let fee = PolyClient::calculate_fee(0.0, 0.50, 100);
    assert_eq!(fee, 0.0);
}
