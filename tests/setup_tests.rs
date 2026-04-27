//! End-to-end tests for the `poly setup` wizard's core logic.
//!
//! The wizard's interactive prompts (rpassword, stdin) aren't easy to drive
//! from a test, but the underlying validation, config-write, and config-read
//! helpers are pure. These tests cover the wizard's full happy path by
//! composing those helpers the same way `setup::run` does.

use poly::setup::{
    has_config, load_existing, validate_eth_address, validate_private_key, validate_url,
    write_config,
};
use std::path::PathBuf;
use tempfile::TempDir;

// ── validate_private_key ─────────────────────────────────────────────────────

#[test]
fn private_key_accepts_0x_prefixed_64_hex() {
    let pk = format!("0x{}", "ab".repeat(32));
    assert_eq!(validate_private_key(&pk).unwrap(), pk);
}

#[test]
fn private_key_auto_prefixes_bare_64_hex() {
    let bare = "ab".repeat(32);
    let normalized = validate_private_key(&bare).unwrap();
    assert_eq!(normalized, format!("0x{}", bare));
}

#[test]
fn private_key_strips_whitespace() {
    let pk = format!("  0x{}  ", "cd".repeat(32));
    assert_eq!(validate_private_key(&pk).unwrap(), pk.trim());
}

#[test]
fn private_key_rejects_empty() {
    assert!(validate_private_key("").is_err());
    assert!(validate_private_key("   ").is_err());
}

#[test]
fn private_key_rejects_wrong_length() {
    assert!(validate_private_key("0xabcdef").is_err());
    assert!(validate_private_key(&format!("0x{}", "ab".repeat(33))).is_err());
}

#[test]
fn private_key_rejects_non_hex() {
    let bad = format!("0x{}", "z".repeat(64));
    assert!(validate_private_key(&bad).is_err());
}

#[test]
fn private_key_rejects_zero_scalar() {
    // All-zero is hex-valid but not a valid secp256k1 scalar.
    let zero = format!("0x{}", "0".repeat(64));
    assert!(validate_private_key(&zero).is_err());
}

#[test]
fn private_key_rejects_above_curve_order() {
    // secp256k1 order n = FFFF...FFFEBAAEDCE6AF48A03BBFD25E8CD0364141
    // 0xFFFF...FFFF (max 256-bit value) is above n and must be rejected.
    let too_big = format!("0x{}", "f".repeat(64));
    assert!(validate_private_key(&too_big).is_err());
}

// ── validate_eth_address ─────────────────────────────────────────────────────

#[test]
fn eth_address_accepts_valid() {
    let addr = "0x".to_string() + &"a".repeat(40);
    assert_eq!(validate_eth_address(&addr).unwrap(), addr);
}

#[test]
fn eth_address_rejects_wrong_length() {
    assert!(validate_eth_address("0xabc").is_err());
    assert!(validate_eth_address(&("0x".to_string() + &"a".repeat(41))).is_err());
}

#[test]
fn eth_address_rejects_missing_prefix() {
    let no_prefix = "a".repeat(42);
    assert!(validate_eth_address(&no_prefix).is_err());
}

#[test]
fn eth_address_rejects_non_hex() {
    let bad = "0x".to_string() + &"z".repeat(40);
    assert!(validate_eth_address(&bad).is_err());
}

// ── validate_url ─────────────────────────────────────────────────────────────

#[test]
fn url_accepts_http_and_https() {
    assert!(validate_url("http://example.com").is_ok());
    assert!(validate_url("https://polygon-mainnet.g.alchemy.com/v2/foo").is_ok());
}

#[test]
fn url_rejects_other_schemes() {
    assert!(validate_url("ftp://example.com").is_err());
    assert!(validate_url("example.com").is_err());
    assert!(validate_url("").is_err());
}

// ── write_config / load_existing round-trip ──────────────────────────────────

fn setup_path(tmp: &TempDir) -> PathBuf {
    tmp.path().join("nested").join("config.toml")
}

#[test]
fn write_then_load_round_trip_with_all_fields() {
    let tmp = TempDir::new().unwrap();
    let path = setup_path(&tmp);

    let pk = format!("0x{}", "ab".repeat(32));
    let funder = "0x".to_string() + &"f".repeat(40);
    write_config(
        &path,
        &pk,
        "api-key",
        "api-secret",
        "api-passphrase",
        Some("https://polygon-mainnet.g.alchemy.com/v2/abc"),
        Some(&funder),
        "",
    )
    .unwrap();

    let loaded = load_existing(&path);
    assert_eq!(loaded.private_key.unwrap(), pk);
    assert_eq!(loaded.api_key.unwrap(), "api-key");
    assert_eq!(loaded.api_secret.unwrap(), "api-secret");
    assert_eq!(loaded.api_passphrase.unwrap(), "api-passphrase");
    assert_eq!(
        loaded.rpc_url.unwrap(),
        "https://polygon-mainnet.g.alchemy.com/v2/abc"
    );
    assert_eq!(loaded.funder_address.unwrap(), funder);
}

#[test]
fn write_then_load_round_trip_optional_fields_omitted() {
    let tmp = TempDir::new().unwrap();
    let path = setup_path(&tmp);

    let pk = format!("0x{}", "12".repeat(32));
    write_config(&path, &pk, "k", "s", "p", None, None, "").unwrap();

    let loaded = load_existing(&path);
    assert_eq!(loaded.private_key.unwrap(), pk);
    assert_eq!(loaded.api_key.unwrap(), "k");
    assert!(loaded.rpc_url.is_none());
    assert!(loaded.funder_address.is_none());
}

#[test]
fn write_creates_parent_directory() {
    let tmp = TempDir::new().unwrap();
    let nested = tmp.path().join("a").join("b").join("c");
    let path = nested.join("config.toml");
    assert!(!nested.exists());

    let pk = format!("0x{}", "00".repeat(32));
    write_config(&path, &pk, "k", "s", "p", None, None, "").unwrap();

    assert!(path.exists());
    assert!(nested.is_dir());
}

#[cfg(unix)]
#[test]
fn write_sets_file_permissions_to_0600() {
    use std::os::unix::fs::PermissionsExt;

    let tmp = TempDir::new().unwrap();
    let path = setup_path(&tmp);

    let pk = format!("0x{}", "00".repeat(32));
    write_config(&path, &pk, "k", "s", "p", None, None, "").unwrap();

    let mode = std::fs::metadata(&path).unwrap().permissions().mode();
    // Mask off file-type bits; only the rwx bits should be set to 0o600.
    assert_eq!(mode & 0o777, 0o600);
}

#[test]
fn load_existing_handles_missing_file() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("does_not_exist.toml");
    let loaded = load_existing(&path);
    assert!(loaded.private_key.is_none());
    assert!(loaded.api_key.is_none());
    assert!(loaded.tui_section.is_empty());
}

#[test]
fn load_existing_preserves_tui_section_for_round_trip() {
    let tmp = TempDir::new().unwrap();
    let path = setup_path(&tmp);

    let pk = format!("0x{}", "ab".repeat(32));
    let tui_section = "\n[tui]\nrefresh_interval_secs = 30\nmax_markets = 1000\n";
    write_config(&path, &pk, "k", "s", "p", None, None, tui_section).unwrap();

    let loaded = load_existing(&path);
    assert!(loaded.tui_section.contains("[tui]"));
    assert!(loaded.tui_section.contains("refresh_interval_secs = 30"));
    assert!(loaded.tui_section.contains("max_markets = 1000"));

    // Round-trip: re-write using the loaded tui_section and verify it survives.
    let path2 = tmp.path().join("config2.toml");
    write_config(&path2, &pk, "k", "s", "p", None, None, &loaded.tui_section).unwrap();
    let loaded2 = load_existing(&path2);
    assert!(loaded2.tui_section.contains("refresh_interval_secs = 30"));
}

// ── full wizard happy path (pure logic only) ─────────────────────────────────

#[test]
fn wizard_happy_path_writes_valid_config_from_user_inputs() {
    let tmp = TempDir::new().unwrap();
    let path = setup_path(&tmp);

    // Simulate the wizard's input → validation → write pipeline.
    let raw_inputs = (
        "ab".repeat(32), // bare private key (will get 0x prefixed)
        "my-api-key".to_string(),
        "my-api-secret".to_string(),
        "my-api-passphrase".to_string(),
        "https://polygon-mainnet.g.alchemy.com/v2/key".to_string(),
        "0x".to_string() + &"f".repeat(40),
    );

    let pk = validate_private_key(&raw_inputs.0).expect("private key valid");
    let rpc = validate_url(&raw_inputs.4).expect("rpc valid");
    let funder = validate_eth_address(&raw_inputs.5).expect("funder valid");

    write_config(
        &path,
        &pk,
        &raw_inputs.1,
        &raw_inputs.2,
        &raw_inputs.3,
        Some(&rpc),
        Some(&funder),
        "",
    )
    .unwrap();

    let loaded = load_existing(&path);
    assert_eq!(
        loaded.private_key.unwrap(),
        format!("0x{}", "ab".repeat(32))
    );
    assert_eq!(loaded.api_key.unwrap(), "my-api-key");
    assert_eq!(loaded.rpc_url.unwrap(), raw_inputs.4);
    assert_eq!(loaded.funder_address.unwrap(), raw_inputs.5);
}

// ── has_config (env-driven) ──────────────────────────────────────────────────
//
// These tests mutate the process environment, so they must run sequentially
// to avoid stomping each other. Each test scopes its env edits via
// `EnvGuard` (RAII) so failures don't leak state to other tests.

struct EnvGuard {
    key: &'static str,
    prev: Option<String>,
}

impl EnvGuard {
    fn set(key: &'static str, value: &str) -> Self {
        let prev = std::env::var(key).ok();
        std::env::set_var(key, value);
        Self { key, prev }
    }
    fn unset(key: &'static str) -> Self {
        let prev = std::env::var(key).ok();
        std::env::remove_var(key);
        Self { key, prev }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        match &self.prev {
            Some(v) => std::env::set_var(self.key, v),
            None => std::env::remove_var(self.key),
        }
    }
}

// All env-mutating tests funnel through one `#[test]` so cargo's parallel
// test runner can't interleave them (env vars are process-global).
#[test]
fn has_config_env_var_precedence() {
    // ── poly_private_key alone is enough ─────────────────────────────────
    let _g1 = EnvGuard::unset("POLY_MARKET_KEY");
    let _g2 = EnvGuard::unset("POLY_CONFIG");
    let _g3 = EnvGuard::set("POLY_PRIVATE_KEY", "0xdeadbeef");
    assert!(has_config(), "POLY_PRIVATE_KEY should make has_config true");
    drop(_g3);

    // ── poly_market_key (legacy) also satisfies it ───────────────────────
    let _g4 = EnvGuard::set("POLY_MARKET_KEY", "0xdeadbeef");
    assert!(has_config(), "POLY_MARKET_KEY should make has_config true");
    drop(_g4);

    // ── poly_config pointing at a real file works ───────────────────────
    let tmp = TempDir::new().unwrap();
    let cfg = tmp.path().join("c.toml");
    let pk = format!("0x{}", "ab".repeat(32));
    write_config(&cfg, &pk, "k", "s", "p", None, None, "").unwrap();
    let _g5 = EnvGuard::set("POLY_CONFIG", cfg.to_str().unwrap());
    assert!(has_config(), "POLY_CONFIG → existing file should pass");
    drop(_g5);

    // ── poly_config pointing at a missing file fails ────────────────────
    let _g6 = EnvGuard::set("POLY_CONFIG", "/nonexistent/path/config.toml");
    let _g7 = EnvGuard::unset("POLY_PRIVATE_KEY");
    let _g8 = EnvGuard::unset("POLY_MARKET_KEY");
    assert!(
        !has_config(),
        "POLY_CONFIG → missing file (and no env keys) should fail"
    );
}
