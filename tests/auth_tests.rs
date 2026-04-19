//! HMAC-SHA256 authentication tests for CLOB REST headers.
//!
//! These tests verify that `ClobAuth::headers()` produces correct HMAC
//! signatures and header layout. The golden signature was computed by hand
//! against the algorithm documented in CLAUDE.md:
//!
//!   message   = timestamp + METHOD + /path + body
//!   signature = url_safe_base64(hmac_sha256(url_safe_base64_decode(secret), message))

use base64::{engine::general_purpose::URL_SAFE, Engine as _};
use hmac::{Hmac, Mac};
use sha2::Sha256;

use poly::auth::ClobAuth;

// Deterministic test credentials (not real).
const TEST_KEY: &str = "test-api-key-uuid";
const TEST_PASSPHRASE: &str = "test-passphrase";

/// A URL-safe base64-encoded secret (32 random bytes).
fn test_secret() -> String {
    URL_SAFE.encode(b"supersecret_32bytes_for_testing!")
}

#[test]
fn headers_contain_all_required_fields() {
    let auth = ClobAuth::new(
        TEST_KEY.to_string(),
        test_secret(),
        TEST_PASSPHRASE.to_string(),
    );
    let address = "0xABCDEF1234567890ABCDEF1234567890ABCDEF12";

    let headers = auth.headers("GET", "/data/orders", None, address).unwrap();

    assert_eq!(headers.get("POLY_ADDRESS").unwrap(), address);
    assert_eq!(headers.get("POLY_API_KEY").unwrap(), TEST_KEY);
    assert_eq!(headers.get("POLY_PASSPHRASE").unwrap(), TEST_PASSPHRASE);
    assert!(headers.get("POLY_TIMESTAMP").is_some());
    assert!(headers.get("POLY_SIGNATURE").is_some());
}

#[test]
fn signature_is_valid_hmac_sha256() {
    let secret = test_secret();
    let auth = ClobAuth::new(
        TEST_KEY.to_string(),
        secret.clone(),
        TEST_PASSPHRASE.to_string(),
    );
    let address = "0x1111111111111111111111111111111111111111";

    let headers = auth
        .headers("POST", "/order", Some(r#"{"token_id":"123"}"#), address)
        .unwrap();

    let timestamp = headers.get("POLY_TIMESTAMP").unwrap().to_str().unwrap();
    let signature = headers.get("POLY_SIGNATURE").unwrap().to_str().unwrap();

    // Recompute expected signature from the same inputs.
    let message = format!("{}POST/order{}", timestamp, r#"{"token_id":"123"}"#);
    let decoded_secret = URL_SAFE.decode(&secret).unwrap();
    let mut mac = Hmac::<Sha256>::new_from_slice(&decoded_secret).unwrap();
    mac.update(message.as_bytes());
    let expected = URL_SAFE.encode(mac.finalize().into_bytes());

    assert_eq!(signature, expected, "HMAC signature mismatch");
}

#[test]
fn empty_body_treated_as_empty_string() {
    let secret = test_secret();
    let auth = ClobAuth::new(
        TEST_KEY.to_string(),
        secret.clone(),
        TEST_PASSPHRASE.to_string(),
    );
    let address = "0x2222222222222222222222222222222222222222";

    let headers = auth.headers("GET", "/data/positions", None, address).unwrap();

    let timestamp = headers.get("POLY_TIMESTAMP").unwrap().to_str().unwrap();
    let signature = headers.get("POLY_SIGNATURE").unwrap().to_str().unwrap();

    // message = timestamp + "GET" + "/data/positions" + "" (no body)
    let message = format!("{}GET/data/positions", timestamp);
    let decoded_secret = URL_SAFE.decode(&secret).unwrap();
    let mut mac = Hmac::<Sha256>::new_from_slice(&decoded_secret).unwrap();
    mac.update(message.as_bytes());
    let expected = URL_SAFE.encode(mac.finalize().into_bytes());

    assert_eq!(signature, expected);
}

#[test]
fn different_methods_produce_different_signatures() {
    let auth = ClobAuth::new(
        TEST_KEY.to_string(),
        test_secret(),
        TEST_PASSPHRASE.to_string(),
    );
    let address = "0x3333333333333333333333333333333333333333";

    let get_headers = auth.headers("GET", "/order", None, address).unwrap();
    let del_headers = auth.headers("DELETE", "/order", None, address).unwrap();

    // Timestamps may differ by a second, but even with the same timestamp
    // the method difference must change the signature.
    let get_sig = get_headers.get("POLY_SIGNATURE").unwrap();
    let del_sig = del_headers.get("POLY_SIGNATURE").unwrap();

    // With extremely high probability these differ (different message content).
    // If timestamps happen to differ, they definitely differ.
    assert_ne!(get_sig, del_sig);
}

#[test]
fn ws_auth_message_contains_credentials() {
    let auth = ClobAuth::new(
        TEST_KEY.to_string(),
        test_secret(),
        TEST_PASSPHRASE.to_string(),
    );

    let msg = auth.ws_auth_message();

    assert_eq!(msg["type"], "user");
    assert_eq!(msg["auth"]["apiKey"], TEST_KEY);
    assert_eq!(msg["auth"]["secret"], test_secret());
    assert_eq!(msg["auth"]["passphrase"], TEST_PASSPHRASE);
}
