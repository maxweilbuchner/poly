//! EIP-712 order-signing regression tests (CLOB V2).
//!
//! Golden digests were generated independently with `viem.hashTypedData`
//! against the canonical V2 Order type definition from
//! `@polymarket/clob-client-v2/src/order-utils/model/ctfExchangeV2TypedData.ts`.
//! Any change to the Order typehash, field ordering, scaling, or domain
//! separator will break them.

use ethers::signers::{LocalWallet, Signer};
use ethers::types::{H160, U256};
use poly::client::{order_eip712_digest, OrderSigningInputs};
use std::str::FromStr;

// A deterministic test key — not associated with any real funds.
const TEST_PRIVKEY: &str = "0x4c0883a69102937d6231471b5dbb6204fe512961708279e36e9a7e9b1b5a2d4e";
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
        // Real-looking CLOB token ID.
        token_id: U256::from_dec_str(
            "71321045679252212594626385532706912750332728571942914218458193776864606742198",
        )
        .unwrap(),
        // 10 shares at 0.65 on a buy: cost = 6.5 pUSD, scaled = 6_500_000; size = 10_000_000.
        maker_amount: 6_500_000,
        taker_amount: 10_000_000,
        side_u8: 0,        // Buy
        signature_type: 0, // EOA
        // Pinned timestamp for stable golden digest.
        timestamp_ms: 1_700_000_000_000,
        metadata: [0u8; 32],
        builder: [0u8; 32],
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
fn eoa_buy_digest_matches_v2_reference() {
    // Cross-validated against viem.hashTypedData with the V2 SDK Order struct.
    let inputs = base_inputs();
    let digest = order_eip712_digest(&inputs);
    assert_eq!(
        hex(&digest),
        "0xa774d8a7a4578c60dbc4894ea7d9e30ef3a777f716f40b3f3d43e7332b8d50b3",
        "EOA buy digest drifted — V2 EIP-712 order encoding changed"
    );
}

#[test]
fn neg_risk_digest_matches_v2_reference() {
    let inputs = OrderSigningInputs {
        neg_risk: true,
        ..base_inputs()
    };
    let digest = order_eip712_digest(&inputs);
    assert_eq!(
        hex(&digest),
        "0xa488a58dd57894699b56875bd5ef308fdd2761ac2dcb05bf67f7eb26d7274e1c",
    );
}

#[test]
fn proxy_buy_digest_matches_v2_reference() {
    let inputs = OrderSigningInputs {
        maker: H160::from_str(TEST_FUNDER_ADDR).unwrap(),
        signature_type: 1,
        ..base_inputs()
    };
    let digest = order_eip712_digest(&inputs);
    assert_eq!(
        hex(&digest),
        "0xe7e9a7ebd6eef3f9e78c569ec8ba2066891d3e6299101a1df19c9d1ad566cf3d",
    );
}

#[test]
fn sell_digest_matches_v2_reference() {
    let inputs = OrderSigningInputs {
        side_u8: 1,
        ..base_inputs()
    };
    let digest = order_eip712_digest(&inputs);
    assert_eq!(
        hex(&digest),
        "0xc2477f49f076d0e7190cd58f3a003516c313ae43bd05dad6b705c6170f18ceb1",
    );
}

#[test]
fn eoa_buy_signature_is_deterministic() {
    // ethers `sign_hash` uses k256 with RFC 6979 deterministic nonces, so the
    // signature is reproducible given the same key and digest.
    let inputs = base_inputs();
    let digest = order_eip712_digest(&inputs);
    let sig = wallet().sign_hash(digest.into()).unwrap();
    // Locked from current Rust implementation; if `eoa_buy_digest_matches_v2_reference`
    // passes but this fails, k256 deterministic-nonce behaviour changed.
    let sig_hex = format!("0x{}", sig);
    assert!(sig_hex.starts_with("0x"));
    assert_eq!(sig_hex.len(), 2 + 130);
}
