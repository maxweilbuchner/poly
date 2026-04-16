//! EIP-712 order-signing regression tests.
//!
//! These tests lock the current `order_eip712_digest` implementation against
//! deterministic golden digests and signatures. Any change to the Order type
//! hash, field ordering, scaling, or domain separator will break them. The
//! golden values were captured from the committed implementation; if they
//! ever need to be regenerated, cross-validate against `py-clob-client`
//! before updating.

use ethers::signers::{LocalWallet, Signer};
use ethers::types::{H160, U256};
use poly::client::{order_eip712_digest, OrderSigningInputs};
use std::str::FromStr;

// A deterministic test key — not associated with any real funds.
const TEST_PRIVKEY: &str =
    "0x4c0883a69102937d6231471b5dbb6204fe512961708279e36e9a7e9b1b5a2d4e";
// Address of TEST_PRIVKEY.
const TEST_SIGNER_ADDR: &str = "0x686C505e1Fc4510A27f13D0fBEAb3aec056b2237";
// A distinct address standing in for a proxy/funder wallet.
const TEST_FUNDER_ADDR: &str = "0x1234567890123456789012345678901234567890";

fn wallet() -> LocalWallet {
    LocalWallet::from_str(TEST_PRIVKEY).unwrap()
}

fn hex(b: &[u8]) -> String {
    format!("0x{}", hex::encode(b))
}

fn base_inputs() -> OrderSigningInputs {
    OrderSigningInputs {
        salt: U256::from(12345u64),
        maker: H160::from_str(TEST_SIGNER_ADDR).unwrap(),
        signer: H160::from_str(TEST_SIGNER_ADDR).unwrap(),
        taker: H160::zero(),
        // Real-looking CLOB token ID.
        token_id: U256::from_dec_str(
            "71321045679252212594626385532706912750332728571942914218458193776864606742198",
        )
        .unwrap(),
        // 10 shares at 0.65 on a buy: cost = 6.5 USDC, scaled = 6_500_000; size = 10_000_000.
        maker_amount: 6_500_000,
        taker_amount: 10_000_000,
        expiration: 0,
        fee_rate_bps: 0,
        side_u8: 0, // Buy
        signature_type: 0, // EOA
        neg_risk: false,
    }
}

#[test]
fn wallet_address_matches_declared_constant() {
    // Sanity: the hardcoded TEST_SIGNER_ADDR must match the derived address,
    // otherwise every subsequent assertion is meaningless.
    let w = wallet();
    assert_eq!(
        format!("{:?}", w.address()).to_lowercase(),
        TEST_SIGNER_ADDR.to_lowercase()
    );
}

#[test]
fn eoa_buy_digest_is_stable() {
    let inputs = base_inputs();
    let digest = order_eip712_digest(&inputs);
    assert_eq!(
        hex(&digest),
        "0x9c5aa7f573e6bb97a6ad25886a7daf597af1de946fac13a7ad3787e3863db09f",
        "EOA buy digest drifted — EIP-712 order encoding changed"
    );
}

#[test]
fn eoa_buy_signature_is_deterministic() {
    // ethers `sign_hash` uses k256 with RFC 6979 deterministic nonces, so the
    // signature is reproducible given the same key and digest.
    let inputs = base_inputs();
    let digest = order_eip712_digest(&inputs);
    let sig = wallet().sign_hash(digest.into()).unwrap();
    assert_eq!(
        format!("0x{}", sig),
        "0x0ec73c4ae37be97826a29311b77edaaeb749688473b8fe6a6cdcaef91e63b05f1119bb09e56eddfb2143adf4eeaa3de2726278c3c3c95d4a97e179cb88a5430f1b"
    );
}

#[test]
fn neg_risk_flag_changes_digest() {
    // Flipping the neg_risk flag must pick a different verifying contract,
    // which must change the domain separator, which must change the digest.
    let mut a = base_inputs();
    a.neg_risk = false;
    let mut b = base_inputs();
    b.neg_risk = true;
    assert_ne!(order_eip712_digest(&a), order_eip712_digest(&b));
}

#[test]
fn proxy_digest_differs_from_eoa() {
    // signature_type and maker != signer both feed into the struct hash.
    let eoa = base_inputs();
    let proxy = OrderSigningInputs {
        maker: H160::from_str(TEST_FUNDER_ADDR).unwrap(),
        signature_type: 1,
        ..base_inputs()
    };
    assert_ne!(order_eip712_digest(&eoa), order_eip712_digest(&proxy));
}

#[test]
fn side_sell_differs_from_buy() {
    // side is encoded as a uint8 in the struct hash; flipping must change digest.
    let buy = base_inputs();
    let sell = OrderSigningInputs { side_u8: 1, ..base_inputs() };
    assert_ne!(order_eip712_digest(&buy), order_eip712_digest(&sell));
}
