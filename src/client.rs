use ethers::providers::{Http, Provider};
use ethers::signers::{LocalWallet, Signer};
use ethers::types::{H160, U256};
use rand::Rng;
use reqwest::Client;
use serde::Deserialize;
use std::str::FromStr;
use std::sync::Arc;
use tokio::sync::Semaphore;

use crate::auth::ClobAuth;
use crate::error::AppError;
use crate::types::{
    Market, MarketStatus, Order, OrderBook, OrderStatus, OrderType, Outcome, PlaceOrderParams,
    Position, PriceLevel, PricePoint, Side,
};

pub type Result<T> = std::result::Result<T, AppError>;

fn net_err(e: reqwest::Error) -> AppError {
    AppError::from_reqwest(e)
}

async fn api_err(resp: reqwest::Response) -> AppError {
    let status = resp.status().as_u16();
    let url = resp.url().to_string();
    let body = resp.text().await.unwrap_or_default();
    tracing::error!(status, url = %url, body = %body, "API error");
    AppError::from_api_body(status, &body)
}

/// Execute a request with up to 3 retries on 429 (rate-limit) or 5xx (transient server error).
/// `build` is called fresh on every attempt since `RequestBuilder` is consumed by `.send()`.
/// Respects the `Retry-After` header when present; otherwise uses exponential backoff (1 s → 2 s → 4 s).
/// On exhaustion the final response is returned as-is so callers can still inspect the status.
async fn send_with_retry<F>(build: F) -> reqwest::Result<reqwest::Response>
where
    F: Fn() -> reqwest::RequestBuilder,
{
    const MAX_ATTEMPTS: u32 = 4;
    let mut delay = std::time::Duration::from_secs(1);
    for attempt in 0..MAX_ATTEMPTS {
        let resp = build().send().await?;
        let status = resp.status().as_u16();
        if (status == 429 || status >= 500) && attempt + 1 < MAX_ATTEMPTS {
            let wait = resp
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.parse::<u64>().ok())
                .map(std::time::Duration::from_secs)
                .unwrap_or(delay);
            tracing::warn!(
                status,
                attempt = attempt + 1,
                wait_ms = wait.as_millis() as u64,
                "retrying request"
            );
            tokio::time::sleep(wait).await;
            delay = (delay * 2).min(std::time::Duration::from_secs(16));
            continue;
        }
        return Ok(resp);
    }
    unreachable!()
}

/// The CLOB paginates some endpoints as `{"data": [...], "next_cursor": "..."}`.
/// This normalises both a bare array and the wrapped form into a plain Vec.
fn extract_data_array(v: serde_json::Value) -> Vec<serde_json::Value> {
    match v {
        serde_json::Value::Array(arr) => arr,
        serde_json::Value::Object(ref obj) => obj
            .get("data")
            .and_then(|d| d.as_array())
            .cloned()
            .unwrap_or_default(),
        _ => vec![],
    }
}

const GAMMA_API: &str = "https://gamma-api.polymarket.com";
const CLOB_API: &str = "https://clob.polymarket.com";
const DATA_API: &str = "https://data-api.polymarket.com";
const CTF_EXCHANGE: &str = "0x4bFb41d5B3570DeFd03C39a9A4D8dE6Bd8B8982E";
const NEG_RISK_CTF_EXCHANGE: &str = "0xC5d563A36AE78145C45a50134d48A1215220f80a";

/// EIP-712 inputs for a Polymarket CTF Exchange order. Amount fields are in
/// USDC micro-units (6 decimals). Exposed for regression testing.
#[derive(Debug, Clone)]
pub struct OrderSigningInputs {
    pub salt: U256,
    pub maker: H160,
    pub signer: H160,
    pub taker: H160,
    pub token_id: U256,
    pub maker_amount: u64,
    pub taker_amount: u64,
    pub expiration: u64,
    pub fee_rate_bps: u64,
    pub side_u8: u8,
    pub signature_type: u8,
    pub neg_risk: bool,
}

/// Compute the EIP-712 digest for a Polymarket CTF Exchange order.
///
/// Returns the 32-byte `keccak256(0x1901 || domainSeparator || structHash)`
/// that `LocalWallet::sign_hash` should be invoked on. Pure function — no
/// randomness, no I/O — so it is directly testable against golden vectors.
pub fn order_eip712_digest(i: &OrderSigningInputs) -> [u8; 32] {
    let type_hash = ethers::utils::keccak256(
        b"Order(uint256 salt,address maker,address signer,address taker,uint256 tokenId,uint256 makerAmount,uint256 takerAmount,uint256 expiration,uint256 nonce,uint256 feeRateBps,uint8 side,uint8 signatureType)",
    );

    let domain_type_hash = ethers::utils::keccak256(
        b"EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)",
    );
    let domain_name_hash = ethers::utils::keccak256(b"Polymarket CTF Exchange");
    let domain_version_hash = ethers::utils::keccak256(b"1");
    let chain_id = U256::from(137u64);
    let exchange_addr = if i.neg_risk {
        NEG_RISK_CTF_EXCHANGE
    } else {
        CTF_EXCHANGE
    };
    let verifying_contract: H160 = exchange_addr.parse().expect("valid CTF exchange address");

    let domain_separator = {
        let mut enc = Vec::with_capacity(5 * 32);
        enc.extend_from_slice(&domain_type_hash);
        enc.extend_from_slice(&domain_name_hash);
        enc.extend_from_slice(&domain_version_hash);
        let mut v = [0u8; 32];
        chain_id.to_big_endian(&mut v);
        enc.extend_from_slice(&v);
        let mut v = [0u8; 32];
        v[12..].copy_from_slice(verifying_contract.as_bytes());
        enc.extend_from_slice(&v);
        ethers::utils::keccak256(enc)
    };

    let struct_hash = {
        let mut enc = Vec::with_capacity(13 * 32);
        enc.extend_from_slice(&type_hash);
        let mut v = [0u8; 32];
        i.salt.to_big_endian(&mut v);
        enc.extend_from_slice(&v);
        let mut v = [0u8; 32];
        v[12..].copy_from_slice(i.maker.as_bytes());
        enc.extend_from_slice(&v);
        let mut v = [0u8; 32];
        v[12..].copy_from_slice(i.signer.as_bytes());
        enc.extend_from_slice(&v);
        let mut v = [0u8; 32];
        v[12..].copy_from_slice(i.taker.as_bytes());
        enc.extend_from_slice(&v);
        let mut v = [0u8; 32];
        i.token_id.to_big_endian(&mut v);
        enc.extend_from_slice(&v);
        let mut v = [0u8; 32];
        U256::from(i.maker_amount).to_big_endian(&mut v);
        enc.extend_from_slice(&v);
        let mut v = [0u8; 32];
        U256::from(i.taker_amount).to_big_endian(&mut v);
        enc.extend_from_slice(&v);
        let mut v = [0u8; 32];
        U256::from(i.expiration).to_big_endian(&mut v);
        enc.extend_from_slice(&v);
        enc.extend_from_slice(&[0u8; 32]); // nonce = 0
        let mut v = [0u8; 32];
        U256::from(i.fee_rate_bps).to_big_endian(&mut v);
        enc.extend_from_slice(&v);
        let mut v = [0u8; 32];
        v[31] = i.side_u8;
        enc.extend_from_slice(&v);
        let mut v = [0u8; 32];
        v[31] = i.signature_type;
        enc.extend_from_slice(&v);
        ethers::utils::keccak256(enc)
    };

    let mut msg = [0u8; 66];
    msg[0] = 0x19;
    msg[1] = 0x01;
    msg[2..34].copy_from_slice(&domain_separator);
    msg[34..66].copy_from_slice(&struct_hash);
    ethers::utils::keccak256(msg)
}
const USDC_ADDRESS: &str = "0x2791Bca1f2de4661ED88A30C99A7a9449Aa84174";
/// Gnosis ConditionalTokens contract — holds ERC-1155 outcome tokens and handles redemption.
const CTF_ADDRESS: &str = "0x4D97DCd97eC945f40cF65F87097ACe5EA0476045";

ethers::contract::abigen!(
    ERC20,
    r#"[
        function balanceOf(address account) external view returns (uint256)
        function allowance(address owner, address spender) external view returns (uint256)
    ]"#
);

ethers::contract::abigen!(
    ConditionalTokens,
    r#"[
        function redeemPositions(address collateralToken, bytes32 parentCollectionId, bytes32 conditionId, uint256[] indexSets) external
    ]"#
);

// ── Raw Gamma API types ───────────────────────────────────────────────────────

#[derive(Deserialize)]
struct GammaMarket {
    #[serde(rename = "conditionId", default)]
    condition_id: String,
    #[serde(default)]
    question: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(rename = "marketSlug", default)]
    market_slug: String,
    #[serde(default)]
    slug: String,
    #[serde(default)]
    active: bool,
    #[serde(default)]
    closed: bool,
    #[serde(rename = "endDate", default)]
    end_date: Option<String>,
    #[serde(default)]
    volume: Option<serde_json::Value>,
    #[serde(default)]
    liquidity: Option<serde_json::Value>,
    #[serde(default)]
    outcomes: Option<String>,
    #[serde(rename = "outcomePrices", default)]
    outcome_prices: Option<String>,
    #[serde(rename = "clobTokenIds", default)]
    clob_token_ids: Option<String>,
    #[serde(rename = "groupItemTitle", default)]
    group_item_title: Option<String>,
    #[serde(rename = "groupSlug", default)]
    group_slug: String,
    #[serde(rename = "category", default)]
    category: Option<String>,
    #[serde(default)]
    tags: Option<Vec<TagEntry>>,
    #[serde(rename = "negRisk", default)]
    neg_risk: bool,
}

#[derive(Deserialize)]
struct TagEntry {
    #[serde(default)]
    label: String,
}

#[derive(Deserialize)]
struct GammaEvent {
    #[serde(default)]
    markets: Vec<GammaMarket>,
}

/// Raw shape of a closed/resolved market from the Gamma API.
/// The Gamma API does not expose a `resolution` string directly; the resolved
/// outcome is instead indicated by the `outcomePrices` array: the winning
/// outcome settles to a price of "1" while losers settle to "0".
#[derive(Deserialize)]
struct GammaClosedMarket {
    #[serde(rename = "conditionId", default)]
    condition_id: String,
    #[serde(default)]
    question: String,
    #[serde(rename = "marketSlug", default)]
    market_slug: String,
    #[serde(default)]
    slug: String,
    #[serde(rename = "endDate", default)]
    end_date: Option<String>,
    /// JSON-encoded outcome names, e.g. `"[\"Yes\", \"No\"]"`.
    #[serde(default)]
    outcomes: Option<String>,
    /// JSON-encoded settlement prices, e.g. `"[\"0\", \"1\"]"`.
    /// Winning outcome has price ≈ 1.0; use `derive_resolution` to extract it.
    #[serde(rename = "outcomePrices", default)]
    outcome_prices: Option<String>,
    /// Last traded YES-outcome price before the market closed.
    #[serde(rename = "lastTradePrice", default)]
    last_trade_price: Option<f64>,
    /// JSON-encoded CLOB token IDs, e.g. `"[\"<yes_id>\", \"<no_id>\"]"`.
    /// The first element is the Yes token used for price history lookups.
    #[serde(rename = "clobTokenIds", default)]
    clob_token_ids: Option<String>,
}

/// A confirmed market resolution as returned by the Gamma API.
#[derive(Debug, Clone)]
pub struct MarketResolution {
    pub condition_id: String,
    pub question: String,
    pub slug: String,
    pub end_date: Option<String>,
    /// The winning outcome name, e.g. "Yes", "No", "Donald Trump".
    pub resolution: String,
    /// Last traded YES-outcome price before the market closed.
    pub last_trade_price: Option<f64>,
    /// CLOB Yes-token ID (first element of `clobTokenIds`).
    pub clob_token_id: Option<String>,
}

// ── Raw CLOB order book types ─────────────────────────────────────────────────

#[derive(Deserialize)]
struct BookEntry {
    price: String,
    size: String,
}

#[derive(Deserialize)]
struct BookResponse {
    #[serde(default)]
    bids: Vec<BookEntry>,
    #[serde(default)]
    asks: Vec<BookEntry>,
}

// ── PolyClient ────────────────────────────────────────────────────────────────

/// Maximum number of API requests the client will have in-flight at once.
/// This prevents the TUI's parallel refresh tasks from overwhelming the
/// Polymarket API and triggering 429 rate-limit responses.
const MAX_CONCURRENT_REQUESTS: usize = 8;

#[derive(Clone)]
pub struct PolyClient {
    http: Client,
    wallet: Option<LocalWallet>,
    funder_address: Option<H160>,
    pub auth: Option<ClobAuth>,
    provider: Option<Provider<Http>>,
    gamma_url: String,
    clob_url: String,
    data_url: String,
    /// Semaphore that limits concurrent HTTP requests to the Polymarket APIs.
    api_semaphore: Arc<Semaphore>,
}

impl PolyClient {
    pub fn new(
        private_key: Option<String>,
        funder_address_str: Option<String>,
        auth: Option<ClobAuth>,
        rpc_url: Option<String>,
    ) -> Self {
        let wallet = private_key.and_then(|k| LocalWallet::from_str(&k).ok());
        let funder_address = funder_address_str.and_then(|a| a.parse::<H160>().ok());
        let provider = rpc_url.and_then(|u| Provider::<Http>::try_from(u).ok());
        let http = Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .expect("HTTP client");
        Self {
            http,
            wallet,
            funder_address,
            auth,
            provider,
            gamma_url: GAMMA_API.to_string(),
            clob_url: CLOB_API.to_string(),
            data_url: DATA_API.to_string(),
            api_semaphore: Arc::new(Semaphore::new(MAX_CONCURRENT_REQUESTS)),
        }
    }

    /// Constructor for tests — overrides base URLs so mock servers can intercept calls.
    #[doc(hidden)]
    #[allow(dead_code)]
    pub fn new_test(gamma_url: &str, clob_url: &str, data_url: &str) -> Self {
        Self {
            http: Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .expect("HTTP client"),
            wallet: None,
            funder_address: None,
            auth: None,
            provider: None,
            gamma_url: gamma_url.to_string(),
            clob_url: clob_url.to_string(),
            data_url: data_url.to_string(),
            api_semaphore: Arc::new(Semaphore::new(MAX_CONCURRENT_REQUESTS)),
        }
    }

    /// Send an HTTP request with retry logic, respecting the concurrency limit.
    /// Acquires a semaphore permit before sending so that at most
    /// [`MAX_CONCURRENT_REQUESTS`] requests are in-flight at any time.
    async fn throttled_send<F>(&self, build: F) -> reqwest::Result<reqwest::Response>
    where
        F: Fn() -> reqwest::RequestBuilder,
    {
        let _permit = self
            .api_semaphore
            .acquire()
            .await
            .expect("semaphore closed");
        send_with_retry(build).await
    }

    // ── Market search / discovery ─────────────────────────────────────────────

    /// Search Polymarket markets by keyword.
    ///
    /// The Gamma API's `search` parameter matches across all metadata (tags,
    /// descriptions, group titles) and returns results sorted by volume, so
    /// unrelated high-volume markets can appear. We fetch a larger batch from
    /// the API and then filter client-side to those whose question or slug
    /// contains the query term (case-insensitive), returning the top `limit`.
    pub async fn search_markets(
        &self,
        query: &str,
        active_only: bool,
        limit: usize,
    ) -> Result<Vec<Market>> {
        let active_param = if active_only {
            "&active=true&closed=false"
        } else {
            ""
        };
        // Fetch a generous over-fetch so client-side filtering has enough to work with
        let fetch_limit = (limit * 8).max(100);
        let url = format!(
            "{}/markets?search={}&limit={}{}",
            self.gamma_url,
            urlencoded(query),
            fetch_limit,
            active_param
        );

        let resp = self
            .throttled_send(|| self.http.get(&url))
            .await
            .map_err(net_err)?;
        if !resp.status().is_success() {
            return Err(api_err(resp).await);
        }

        let raw: Vec<GammaMarket> = resp.json().await?;
        let q_lower = query.to_lowercase();

        let mut markets = Vec::new();
        for gm in raw {
            // Filter: question or slug must contain the search term
            let q_match = gm.question.to_lowercase().contains(&q_lower);
            let s_match = gm.market_slug.to_lowercase().contains(&q_lower);
            let g_match = gm
                .group_item_title
                .as_deref()
                .map(|g| g.to_lowercase().contains(&q_lower))
                .unwrap_or(false);

            if !q_match && !s_match && !g_match {
                continue;
            }

            if let Some(m) = self.gamma_to_market(gm, false).await {
                markets.push(m);
                if markets.len() >= limit {
                    break;
                }
            }
        }
        Ok(markets)
    }

    /// Fetch top markets by volume (active only).
    ///
    /// Over-fetches from the Gamma API (which returns by volume desc by default)
    /// and drops any markets that fail to deserialise, returning up to `limit` results.
    pub async fn get_top_markets(
        &self,
        limit: usize,
        category: Option<&str>,
    ) -> Result<Vec<Market>> {
        // Over-fetch so category filtering still yields `limit` results.
        let fetch_limit = if category.is_some() {
            (limit * 4).max(100)
        } else {
            limit
        };
        let url = format!(
            "{}/markets?active=true&closed=false&order=volume&ascending=false&limit={}",
            self.gamma_url, fetch_limit
        );

        let resp = self
            .throttled_send(|| self.http.get(&url))
            .await
            .map_err(net_err)?;
        if !resp.status().is_success() {
            return Err(api_err(resp).await);
        }

        let raw: Vec<GammaMarket> = resp.json().await?;
        let cat_lower = category.map(|c| c.to_lowercase());

        let mut markets = Vec::new();
        for gm in raw {
            if let Some(ref cat) = cat_lower {
                let gm_cat = gm.category.as_deref().unwrap_or("").to_lowercase();
                if !gm_cat.contains(cat.as_str()) {
                    continue;
                }
            }
            if let Some(m) = self.gamma_to_market(gm, false).await {
                markets.push(m);
                if markets.len() >= limit {
                    break;
                }
            }
        }
        Ok(markets)
    }

    /// Fetch one page of active markets sorted by volume descending.
    /// `offset` is the number of markets to skip (for pagination).
    pub async fn get_markets_page(&self, offset: usize, limit: usize) -> Result<Vec<Market>> {
        let url = format!(
            "{}/markets?active=true&closed=false&order=volume&ascending=false&limit={}&offset={}",
            self.gamma_url, limit, offset
        );

        let resp = self
            .throttled_send(|| self.http.get(&url))
            .await
            .map_err(net_err)?;
        if !resp.status().is_success() {
            return Err(api_err(resp).await);
        }

        let raw: Vec<GammaMarket> = resp.json().await?;
        let mut markets = Vec::new();
        for gm in raw {
            if let Some(m) = self.gamma_to_market(gm, false).await {
                markets.push(m);
            }
        }
        Ok(markets)
    }

    /// Fetch one page of all active markets (no end-date filter) for snapshots.
    pub async fn get_markets_page_snapshot(
        &self,
        offset: usize,
        limit: usize,
    ) -> Result<Vec<Market>> {
        let url = format!(
            "{}/markets?active=true&closed=false&order=volume&ascending=false&limit={}&offset={}",
            self.gamma_url, limit, offset
        );
        let resp = self
            .throttled_send(|| self.http.get(&url))
            .await
            .map_err(net_err)?;
        if !resp.status().is_success() {
            return Err(api_err(resp).await);
        }
        let raw: Vec<GammaMarket> = resp.json().await?;
        let mut markets = Vec::new();
        for gm in raw {
            if let Some(m) = self.gamma_to_market(gm, false).await {
                markets.push(m);
            }
        }
        Ok(markets)
    }

    /// Fetch one page of closed markets ordered by end date descending, keeping
    /// only those that carry an explicit `resolution` value (confirmed by Polymarket).
    pub async fn get_resolved_markets_page(
        &self,
        offset: usize,
        limit: usize,
    ) -> Result<Vec<MarketResolution>> {
        let url = format!(
            "{}/markets?closed=true&order=endDate&ascending=false&limit={}&offset={}",
            self.gamma_url, limit, offset
        );
        let resp = self
            .throttled_send(|| self.http.get(&url))
            .await
            .map_err(net_err)?;
        if !resp.status().is_success() {
            return Err(api_err(resp).await);
        }
        let raw: Vec<GammaClosedMarket> = resp.json().await?;
        let resolved = raw
            .into_iter()
            .filter_map(|m| {
                let resolution =
                    derive_resolution(m.outcomes.as_deref(), m.outcome_prices.as_deref())?;
                let slug = if m.slug.is_empty() {
                    m.market_slug
                } else {
                    m.slug
                };
                let clob_token_id = parse_yes_token_id(m.clob_token_ids.as_deref());
                Some(MarketResolution {
                    condition_id: m.condition_id,
                    question: m.question,
                    slug,
                    end_date: m.end_date,
                    resolution,
                    last_trade_price: m.last_trade_price,
                    clob_token_id,
                })
            })
            .collect();
        Ok(resolved)
    }

    /// Fetch a single market by condition ID and return its resolution if confirmed.
    /// Returns `Ok(None)` when the market exists but has no resolution yet.
    pub async fn get_market_resolution(
        &self,
        condition_id: &str,
    ) -> Result<Option<MarketResolution>> {
        let url = format!("{}/markets/{}", self.gamma_url, condition_id);
        let resp = self
            .throttled_send(|| self.http.get(&url))
            .await
            .map_err(net_err)?;
        if resp.status().as_u16() == 404 {
            return Ok(None);
        }
        if !resp.status().is_success() {
            return Err(api_err(resp).await);
        }
        let m: GammaClosedMarket = resp.json().await?;
        let resolution = match derive_resolution(m.outcomes.as_deref(), m.outcome_prices.as_deref())
        {
            Some(r) => r,
            None => return Ok(None),
        };
        let slug = if m.slug.is_empty() {
            m.market_slug
        } else {
            m.slug
        };
        let clob_token_id = parse_yes_token_id(m.clob_token_ids.as_deref());
        Ok(Some(MarketResolution {
            condition_id: m.condition_id,
            question: m.question,
            slug,
            end_date: m.end_date,
            resolution,
            last_trade_price: m.last_trade_price,
            clob_token_id,
        }))
    }

    /// Fetch a single market by its condition ID (hex string).
    pub async fn get_market_by_id(&self, condition_id: &str) -> Result<Option<Market>> {
        let url = format!("{}/markets/{}", self.gamma_url, condition_id);
        let resp = self
            .throttled_send(|| self.http.get(&url))
            .await
            .map_err(net_err)?;
        if resp.status().as_u16() == 404 {
            return Ok(None);
        }
        if !resp.status().is_success() {
            return Err(api_err(resp).await);
        }
        let gm: GammaMarket = resp.json().await?;
        Ok(self.gamma_to_market(gm, true).await)
    }

    /// Look up a market by one of its CLOB token IDs (useful for neg-risk markets
    /// whose condition ID may not be directly queryable via `/markets/{id}`).
    pub async fn get_market_by_token_id(&self, token_id: &str) -> Result<Option<Market>> {
        let url = format!(
            "{}/markets?clobTokenIds={}&limit=1",
            self.gamma_url, token_id
        );
        let resp = self
            .throttled_send(|| self.http.get(&url))
            .await
            .map_err(net_err)?;
        if !resp.status().is_success() {
            return Err(api_err(resp).await);
        }
        let markets: Vec<GammaMarket> = resp.json().await?;
        match markets.into_iter().next() {
            Some(gm) => Ok(self.gamma_to_market(gm, true).await),
            None => Ok(None),
        }
    }

    /// Look up a market by its question text (useful for neg-risk markets where
    /// both condition ID and token ID lookups fail).
    /// Searches the Gamma API and matches the result by exact question.
    pub async fn get_market_by_question(&self, question: &str) -> Result<Option<Market>> {
        // Strip non-ASCII (e.g. °) for the search query — the Gamma search
        // engine works best with plain ASCII terms.
        let ascii_query: String = question
            .chars()
            .filter(|c| c.is_ascii())
            .collect::<String>()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        let search_term: String = ascii_query.chars().take(80).collect();
        if search_term.is_empty() {
            return Ok(None);
        }
        let url = format!(
            "{}/markets?search={}&limit=20",
            self.gamma_url,
            urlencoded(&search_term)
        );
        let resp = self
            .throttled_send(|| self.http.get(&url))
            .await
            .map_err(net_err)?;
        if !resp.status().is_success() {
            return Err(api_err(resp).await);
        }
        let markets: Vec<GammaMarket> = resp.json().await?;
        // Find the market whose question matches exactly
        for gm in markets {
            if gm.question == question {
                return Ok(self.gamma_to_market(gm, true).await);
            }
        }
        Ok(None)
    }

    /// Fetch a market (or markets within an event) by slug.
    /// Tries the /events endpoint first (event slug), then /markets directly.
    pub async fn get_market_by_slug(&self, slug: &str) -> Result<Vec<Market>> {
        // Try as an event slug first
        let url = format!("{}/events?slug={}", self.gamma_url, urlencoded(slug));
        let resp = self
            .throttled_send(|| self.http.get(&url))
            .await
            .map_err(net_err)?;
        if resp.status().is_success() {
            let events: Vec<GammaEvent> = resp.json().await?;
            let mut markets = Vec::new();
            for event in events {
                for gm in event.markets {
                    if let Some(m) = self.gamma_to_market(gm, true).await {
                        markets.push(m);
                    }
                }
            }
            if !markets.is_empty() {
                return Ok(markets);
            }
        }

        // Try as a direct market slug (correct param name is `slug`, not `market_slug`)
        let url2 = format!("{}/markets?slug={}", self.gamma_url, urlencoded(slug));
        let resp2 = self
            .throttled_send(|| self.http.get(&url2))
            .await
            .map_err(net_err)?;
        if resp2.status().is_success() {
            let raw: Vec<GammaMarket> = resp2.json().await?;
            let mut markets = Vec::new();
            for gm in raw {
                if let Some(m) = self.gamma_to_market(gm, true).await {
                    markets.push(m);
                }
            }
            return Ok(markets);
        }

        Ok(vec![])
    }

    // ── Order book ────────────────────────────────────────────────────────────

    /// Fetch the full order book for a token ID.
    pub async fn get_order_book(&self, token_id: &str) -> Result<OrderBook> {
        let url = format!("{}/book?token_id={}", self.clob_url, token_id);
        let resp = self
            .throttled_send(|| self.http.get(&url))
            .await
            .map_err(net_err)?;
        if !resp.status().is_success() {
            return Err(api_err(resp).await);
        }
        let book: BookResponse = resp.json().await?;

        let parse_levels = |entries: Vec<BookEntry>| -> Vec<PriceLevel> {
            let mut levels: Vec<PriceLevel> = entries
                .into_iter()
                .filter_map(|e| {
                    let price = e.price.parse::<f64>().ok()?;
                    let size = e.size.parse::<f64>().ok()?;
                    Some(PriceLevel { price, size })
                })
                .collect();
            levels.sort_by(|a, b| b.price.total_cmp(&a.price));
            levels
        };

        let mut bids = parse_levels(book.bids);
        let mut asks = parse_levels(book.asks);
        // bids: highest first, asks: lowest first
        bids.sort_by(|a, b| b.price.total_cmp(&a.price));
        asks.sort_by(|a, b| a.price.total_cmp(&b.price));

        Ok(OrderBook {
            token_id: token_id.to_string(),
            bids,
            asks,
        })
    }

    // ── Account info ──────────────────────────────────────────────────────────

    /// USDC balance on-chain (Polygon mainnet).
    pub async fn get_balance(&self) -> Result<f64> {
        let provider = self
            .provider
            .as_ref()
            .ok_or_else(|| AppError::Auth("No RPC URL configured (POLYGON_RPC_URL)".into()))?;
        let address = self
            .funder_address
            .or_else(|| self.wallet.as_ref().map(|w| w.address()))
            .ok_or_else(|| {
                AppError::Auth("No wallet or funder address configured (POLY_PRIVATE_KEY)".into())
            })?;

        let usdc: H160 = USDC_ADDRESS.parse().map_err(AppError::other)?;
        let contract = ERC20::new(usdc, provider.clone().into());
        let raw: U256 = contract
            .balance_of(address)
            .call()
            .await
            .map_err(AppError::other)?;
        Ok(raw.to_string().parse::<f64>().unwrap_or(0.0) / 1_000_000.0)
    }

    /// USDC allowance granted to the CTF Exchange.
    pub async fn get_allowance(&self) -> Result<f64> {
        let provider = self
            .provider
            .as_ref()
            .ok_or_else(|| AppError::Auth("No RPC URL configured (POLYGON_RPC_URL)".into()))?;
        let address = self
            .funder_address
            .or_else(|| self.wallet.as_ref().map(|w| w.address()))
            .ok_or_else(|| {
                AppError::Auth("No wallet or funder address configured (POLY_PRIVATE_KEY)".into())
            })?;

        let usdc: H160 = USDC_ADDRESS.parse().map_err(AppError::other)?;
        let spender: H160 = CTF_EXCHANGE.parse().map_err(AppError::other)?;
        let contract = ERC20::new(usdc, provider.clone().into());
        let raw: U256 = contract
            .allowance(address, spender)
            .call()
            .await
            .map_err(AppError::other)?;
        Ok(raw.to_string().parse::<f64>().unwrap_or(0.0) / 1_000_000.0)
    }

    // ── Orders ────────────────────────────────────────────────────────────────

    /// List open orders for the authenticated user.
    pub async fn get_open_orders(&self) -> Result<Vec<Order>> {
        let (auth, wallet) = self.require_auth()?;
        let sign_path = "/data/orders";
        let url = format!("{}{}", self.clob_url, "/data/orders?status=live");
        let signer_str = format!("{:#x}", wallet.address());
        let headers = auth.headers("GET", sign_path, None, &signer_str)?;

        let resp = self
            .throttled_send(|| self.http.get(&url).headers(headers.clone()))
            .await
            .map_err(net_err)?;
        if !resp.status().is_success() {
            return Err(api_err(resp).await);
        }

        #[derive(Deserialize)]
        struct RawOrder {
            id: Option<String>,
            #[serde(rename = "asset_id", default)]
            asset_id: String,
            side: Option<String>,
            price: Option<serde_json::Value>,
            #[serde(rename = "original_size", default)]
            original_size: Option<serde_json::Value>,
            #[serde(rename = "size_matched", default)]
            size_matched: Option<serde_json::Value>,
            status: Option<String>,
            outcome: Option<String>,
            market: Option<String>,
            #[serde(rename = "created_at", default)]
            created_at: Option<String>,
        }

        // CLOB may return a bare array or a paginated {"data": [...], "next_cursor": "..."}.
        let body: serde_json::Value = resp.json().await?;
        let arr = extract_data_array(body);
        let raw: Vec<RawOrder> =
            serde_json::from_value(serde_json::Value::Array(arr)).unwrap_or_default();
        Ok(raw
            .into_iter()
            .filter_map(|o| {
                Some(Order {
                    id: o.id?,
                    asset_id: o.asset_id,
                    side: match o.side?.as_str() {
                        "BUY" => Side::Buy,
                        "SELL" => Side::Sell,
                        _ => return None,
                    },
                    price: parse_value_f64(&o.price).unwrap_or(0.0),
                    original_size: parse_value_f64(&o.original_size).unwrap_or(0.0),
                    size_matched: parse_value_f64(&o.size_matched).unwrap_or(0.0),
                    status: parse_order_status(o.status.as_deref()),
                    outcome: o.outcome.unwrap_or_default(),
                    market: o.market.unwrap_or_default(),
                    created_at: o.created_at.unwrap_or_default(),
                })
            })
            .collect())
    }

    /// Fetch filled order history for the authenticated user.
    ///
    /// Hits `/data/orders?status=matched` and returns up to `limit` orders,
    /// most recent first (as returned by the CLOB API).
    pub async fn get_order_history(&self, limit: usize) -> Result<Vec<Order>> {
        let (auth, wallet) = self.require_auth()?;
        let sign_path = "/data/orders";
        let url = format!("{}{}", self.clob_url, "/data/orders?status=matched");
        let signer_str = format!("{:#x}", wallet.address());
        let headers = auth.headers("GET", sign_path, None, &signer_str)?;

        let resp = self
            .throttled_send(|| self.http.get(&url).headers(headers.clone()))
            .await
            .map_err(net_err)?;
        if !resp.status().is_success() {
            return Err(api_err(resp).await);
        }

        #[derive(Deserialize)]
        struct RawOrder {
            id: Option<String>,
            #[serde(rename = "asset_id", default)]
            asset_id: String,
            side: Option<String>,
            price: Option<serde_json::Value>,
            #[serde(rename = "original_size", default)]
            original_size: Option<serde_json::Value>,
            #[serde(rename = "size_matched", default)]
            size_matched: Option<serde_json::Value>,
            status: Option<String>,
            outcome: Option<String>,
            market: Option<String>,
            #[serde(rename = "created_at", default)]
            created_at: Option<String>,
        }

        let body: serde_json::Value = resp.json().await?;
        let arr = extract_data_array(body);
        let raw: Vec<RawOrder> =
            serde_json::from_value(serde_json::Value::Array(arr)).unwrap_or_default();
        Ok(raw
            .into_iter()
            .filter_map(|o| {
                Some(Order {
                    id: o.id?,
                    asset_id: o.asset_id,
                    side: match o.side?.as_str() {
                        "BUY" => Side::Buy,
                        "SELL" => Side::Sell,
                        _ => return None,
                    },
                    price: parse_value_f64(&o.price).unwrap_or(0.0),
                    original_size: parse_value_f64(&o.original_size).unwrap_or(0.0),
                    size_matched: parse_value_f64(&o.size_matched).unwrap_or(0.0),
                    status: parse_order_status(o.status.as_deref()),
                    outcome: o.outcome.unwrap_or_default(),
                    market: o.market.unwrap_or_default(),
                    created_at: o.created_at.unwrap_or_default(),
                })
            })
            .take(limit)
            .collect())
    }

    /// Get a single order by ID (raw JSON for inspection).
    #[allow(dead_code)]
    pub async fn get_order(&self, order_id: &str) -> Result<serde_json::Value> {
        let (auth, wallet) = self.require_auth()?;
        let path = format!("/data/order/{}", order_id);
        let url = format!("{}{}", self.clob_url, path);
        let signer_str = format!("{:#x}", wallet.address());
        let headers = auth.headers("GET", &path, None, &signer_str)?;

        let resp = self
            .throttled_send(|| self.http.get(&url).headers(headers.clone()))
            .await
            .map_err(net_err)?;
        if !resp.status().is_success() {
            return Err(api_err(resp).await);
        }
        Ok(resp.json().await?)
    }

    // ── Positions ─────────────────────────────────────────────────────────────

    /// List current positions for the authenticated user.
    pub async fn get_positions(&self) -> Result<Vec<Position>> {
        let (_auth, wallet) = self.require_auth()?;
        let address = self.funder_address.unwrap_or(wallet.address());
        let address_str = format!("{:#x}", address);

        let url = format!(
            "{}/positions?user={}&sizeThreshold=0.1&limit=500",
            self.data_url, address_str
        );
        let resp = self
            .throttled_send(|| self.http.get(&url))
            .await
            .map_err(net_err)?;
        if !resp.status().is_success() {
            return Err(api_err(resp).await);
        }

        #[derive(Deserialize)]
        struct RawPosition {
            #[serde(rename = "asset", default)]
            asset: String,
            #[serde(rename = "conditionId", default)]
            condition_id: String,
            #[serde(rename = "outcome", default)]
            outcome: Option<String>,
            #[serde(rename = "title", default)]
            title: Option<String>,
            #[serde(rename = "size", default)]
            size: Option<serde_json::Value>,
            #[serde(rename = "avgPrice", default)]
            avg_price: Option<serde_json::Value>,
            #[serde(rename = "curPrice", default)]
            cur_price: Option<serde_json::Value>,
            #[serde(rename = "realizedPnl", default)]
            realized_pnl: Option<serde_json::Value>,
            #[serde(rename = "cashPnl", default)]
            cash_pnl: Option<serde_json::Value>,
        }

        let raw: Vec<RawPosition> = resp.json().await?;
        let mut positions: Vec<Position> = raw
            .into_iter()
            .map(|p| Position {
                market_id: p.condition_id,
                market_question: p.title.unwrap_or_default(),
                outcome: p.outcome.unwrap_or_default(),
                token_id: p.asset,
                size: parse_value_f64(&p.size).unwrap_or(0.0),
                avg_price: parse_value_f64(&p.avg_price).unwrap_or(0.0),
                current_price: parse_value_f64(&p.cur_price).unwrap_or(0.0),
                realized_pnl: parse_value_f64(&p.realized_pnl).unwrap_or(0.0),
                unrealized_pnl: parse_value_f64(&p.cash_pnl).unwrap_or(0.0),
                end_date: None,
                neg_risk: false,
                market_closed: false,
                redeemable: false,
            })
            .collect();

        // Fetch end_dates for each unique market concurrently.
        // Many positions are for expired/low-volume markets not in the cached market list,
        // so we resolve them directly from the Gamma API here.
        use futures_util::future::join_all;
        use std::collections::{HashMap, HashSet};

        let unique_ids: Vec<String> = positions
            .iter()
            .map(|p| p.market_id.clone())
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();

        let futs: Vec<_> = unique_ids
            .iter()
            .map(|cid| {
                let url = format!("{}/markets/{}", self.gamma_url, cid);
                let http = self.http.clone();
                let sem = self.api_semaphore.clone();
                let cid = cid.clone();
                async move {
                    let result: Option<(Option<String>, bool, bool)> = async {
                        let _permit = sem.acquire().await.expect("semaphore closed");
                        let r = send_with_retry(|| http.get(&url)).await.ok()?;
                        if !r.status().is_success() {
                            return None;
                        }
                        let gm: GammaMarket = r.json().await.ok()?;
                        Some((gm.end_date, gm.neg_risk, gm.closed))
                    }
                    .await;
                    let (end_date, neg_risk, closed) = result.unwrap_or((None, false, false));
                    (cid, end_date, neg_risk, closed)
                }
            })
            .collect();

        let market_map: HashMap<String, (Option<String>, bool, bool)> = join_all(futs)
            .await
            .into_iter()
            .map(|(cid, end_date, neg_risk, closed)| (cid, (end_date, neg_risk, closed)))
            .collect();

        for p in &mut positions {
            if let Some((end_date, neg_risk, closed)) = market_map.get(&p.market_id) {
                p.end_date = end_date.clone();
                p.neg_risk = *neg_risk;
                p.market_closed = *closed;
                // A position is redeemable when the market is closed (resolved) and this
                // outcome won (curPrice → 1.0).  neg-risk markets use a different adapter
                // contract not yet supported here.
                p.redeemable = *closed && p.current_price > 0.95 && !*neg_risk;
            }
        }

        Ok(positions)
    }

    /// After a failed placement attempt, check whether the order actually landed.
    /// Looks in live orders first (GTC), then matched (FOK fills).
    /// Matches on token_id + side + price + size so we don't false-positive on an
    /// unrelated order with the same asset/side that was placed around the same time.
    /// Returns the order ID if a matching order is found.
    pub async fn find_recent_order(
        &self,
        token_id: &str,
        side: &Side,
        price: f64,
        size: f64,
    ) -> Result<String> {
        let matches = |o: &crate::types::Order| -> bool {
            o.asset_id == token_id
                && &o.side == side
                && (o.price - price).abs() < 1e-6
                && (o.original_size - size).abs() < 1e-4
        };
        if let Ok(orders) = self.get_open_orders().await {
            if let Some(o) = orders.iter().find(|o| matches(o)) {
                return Ok(o.id.clone());
            }
        }
        if let Ok(orders) = self.get_order_history(20).await {
            if let Some(o) = orders.iter().find(|o| matches(o)) {
                return Ok(o.id.clone());
            }
        }
        Err(AppError::Other("no matching order found".into()))
    }

    // ── Order placement ───────────────────────────────────────────────────────

    /// Place a limit order on the CLOB.
    ///
    /// `token_id` is the CLOB token ID for the specific outcome.
    /// `params.price` is in [0.01, 0.99].
    /// `params.size` is the number of shares (min 5, min $1 USDC).
    /// Returns the CLOB `orderID` on success.
    #[tracing::instrument(skip(self), fields(token_id = %params.token_id, side = ?params.side, price = params.price, size = params.size))]
    pub async fn place_order(&self, params: &PlaceOrderParams) -> Result<String> {
        let token_id = params.token_id.as_str();
        let price = params.price;
        let size = params.size;
        let side = params.side.clone();
        let order_type = params.order_type;
        let expiry = params.expiry;
        let neg_risk = params.neg_risk;
        let fee_rate_bps = self.get_fee_rate(token_id).await.ok_or_else(|| {
            AppError::Network("fee rate fetch failed — cannot place order safely".to_string())
        })?;
        let (auth, wallet) = self.require_auth()?;

        let salt = {
            let mut rng = rand::thread_rng();
            U256::from(rng.gen_range(1u64..=9_007_199_254_740_991u64))
        };

        let token_id_u256 = U256::from_dec_str(token_id)
            .map_err(|_| format!("Invalid token_id (must be decimal integer): {}", token_id))?;

        let signer = wallet.address();
        let maker = self.funder_address.unwrap_or(signer);
        let taker = H160::zero();

        // Round to CLOB-acceptable precision.
        // Market buy (FOK+Buy): maker=USDC cost (max 2dp), taker=shares (max 5dp).
        // Limit orders (GTC/IOC) and sells: cost up to 4dp is accepted.
        let rounded_size = (size * 100.0).round() / 100.0;
        let cost_dp = if order_type == OrderType::Fok && side == Side::Buy {
            100.0
        } else {
            10_000.0
        };
        let rounded_cost = (rounded_size * price * cost_dp).round() / cost_dp;

        let decimals = 1_000_000_f64;
        let size_scaled = (rounded_size * decimals).round() as u64;
        let cost_scaled = (rounded_cost * decimals).round() as u64;

        let (maker_amount, taker_amount, side_u8) = match side {
            Side::Buy => (cost_scaled, size_scaled, 0u8),
            Side::Sell => (size_scaled, cost_scaled, 1u8),
        };

        // 0 = EOA (maker == signer), 1 = proxy wallet (funder/maker != signer EOA)
        let signature_type: u8 =
            if self.funder_address.is_some() && self.funder_address != Some(signer) {
                1
            } else {
                0
            };

        let digest = order_eip712_digest(&OrderSigningInputs {
            salt,
            maker,
            signer,
            taker,
            token_id: token_id_u256,
            maker_amount,
            taker_amount,
            expiration: expiry.unwrap_or(0),
            fee_rate_bps,
            side_u8,
            signature_type,
            neg_risk,
        });

        let signature = wallet.sign_hash(digest.into()).map_err(AppError::other)?;
        let sig_str = format!("0x{}", signature);

        let body = serde_json::json!({
            "order": {
                "salt": salt.as_u64(),
                "maker": format!("{:#x}", maker),
                "signer": format!("{:#x}", signer),
                "taker": format!("{:#x}", taker),
                "tokenId": token_id_u256.to_string(),
                "makerAmount": U256::from(maker_amount).to_string(),
                "takerAmount": U256::from(taker_amount).to_string(),
                "expiration": expiry.unwrap_or(0).to_string(),
                "nonce": "0",
                "feeRateBps": fee_rate_bps.to_string(),
                "side": match side { Side::Buy => "BUY", Side::Sell => "SELL" },
                "signatureType": signature_type,
                "signature": sig_str
            },
            "owner": auth.key,
            "orderType": order_type.to_string(),
            "deferExec": false
        });

        let body_str = body.to_string();
        // POLY_ADDRESS is always the EOA signer — that's what the API key is registered to.
        let signer_str = format!("{:#x}", signer);
        let mut headers = auth.headers("POST", "/order", Some(&body_str), &signer_str)?;
        headers.insert(
            "Content-Type",
            "application/json".parse().map_err(AppError::other)?,
        );

        let clob_url = self.clob_url.clone();
        let resp = self
            .throttled_send(|| {
                self.http
                    .post(format!("{}/order", clob_url))
                    .headers(headers.clone())
                    .body(body_str.clone())
            })
            .await
            .map_err(net_err)?;

        if !resp.status().is_success() {
            return Err(api_err(resp).await);
        }

        let json: serde_json::Value = resp.json().await?;

        // Polymarket can return HTTP 200 with success:false for application-level rejections.
        if json.get("success").and_then(|v| v.as_bool()) == Some(false) {
            let msg = json
                .get("errorMsg")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .unwrap_or("order rejected by exchange")
                .to_string();
            return Err(AppError::Api {
                status: 200,
                message: msg,
            });
        }

        let order_id = json
            .get("orderID")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        Ok(order_id)
    }

    /// Cancel a single open order by its CLOB order ID.
    #[tracing::instrument(skip(self))]
    pub async fn cancel_order(&self, order_id: &str) -> Result<()> {
        let (auth, wallet) = self.require_auth()?;
        let body = serde_json::json!({ "orderID": order_id });
        let body_str = body.to_string();
        let signer_str = format!("{:#x}", wallet.address());
        let mut headers = auth.headers("DELETE", "/order", Some(&body_str), &signer_str)?;
        headers.insert(
            "Content-Type",
            "application/json".parse().map_err(AppError::other)?,
        );

        let clob_url = self.clob_url.clone();
        let resp = self
            .throttled_send(|| {
                self.http
                    .delete(format!("{}/order", clob_url))
                    .headers(headers.clone())
                    .body(body_str.clone())
            })
            .await
            .map_err(net_err)?;

        if !resp.status().is_success() {
            return Err(api_err(resp).await);
        }
        Ok(())
    }

    /// Cancel all open orders for the authenticated user.
    pub async fn cancel_all_orders(&self) -> Result<()> {
        let (auth, wallet) = self.require_auth()?;
        let signer_str = format!("{:#x}", wallet.address());
        let headers = auth.headers("DELETE", "/orders", None, &signer_str)?;

        let clob_url = self.clob_url.clone();
        let resp = self
            .throttled_send(|| {
                self.http
                    .delete(format!("{}/orders", clob_url))
                    .headers(headers.clone())
            })
            .await
            .map_err(net_err)?;

        if !resp.status().is_success() {
            return Err(api_err(resp).await);
        }
        Ok(())
    }

    // ── Redemption ───────────────────────────────────────────────────────────

    /// Redeem a resolved winning position on-chain via the Gnosis ConditionalTokens contract.
    ///
    /// Sends `redeemPositions(USDC, 0x0, conditionId, [1, 2])` from the wallet.
    /// Returns the transaction hash on success.
    ///
    /// Requires `POLY_PRIVATE_KEY` and `POLYGON_RPC_URL` to be set.
    /// If a proxy/funder address is configured, the tokens are held by that address
    /// and cannot be redeemed directly from the EOA — returns an error in that case.
    pub async fn redeem_position(&self, condition_id: &str) -> Result<String> {
        let wallet = self.wallet.as_ref().ok_or_else(|| {
            AppError::Auth("POLY_PRIVATE_KEY required for on-chain redemption".into())
        })?;
        let provider = self.provider.as_ref().ok_or_else(|| {
            AppError::Auth("POLYGON_RPC_URL required for on-chain redemption".into())
        })?;

        // If a distinct funder/proxy address is configured, the CTF tokens live there
        // and the EOA cannot redeem directly.
        if let Some(funder) = self.funder_address {
            if funder != wallet.address() {
                return Err(AppError::Other(
                    "Redemption with a proxy/funder address is not yet supported — \
                     redeem on polymarket.com instead"
                        .into(),
                ));
            }
        }

        // Parse conditionId hex string → bytes32.
        let hex = condition_id.trim_start_matches("0x");
        let decoded = hex::decode(hex).map_err(AppError::other)?;
        let mut cid_bytes = [0u8; 32];
        let offset = 32usize.saturating_sub(decoded.len());
        cid_bytes[offset..].copy_from_slice(&decoded[..decoded.len().min(32)]);

        let ctf_addr: H160 = CTF_ADDRESS.parse().map_err(AppError::other)?;
        let collateral: H160 = USDC_ADDRESS.parse().map_err(AppError::other)?;

        use ethers::middleware::SignerMiddleware;
        let signer = SignerMiddleware::new(provider.clone(), wallet.clone().with_chain_id(137u64));
        let client = std::sync::Arc::new(signer);
        let ctf = ConditionalTokens::new(ctf_addr, client);

        let parent = [0u8; 32];
        let index_sets = vec![U256::from(1u64), U256::from(2u64)];

        let call = ctf.redeem_positions(collateral, parent, cid_bytes, index_sets);
        let pending = call.send().await.map_err(AppError::other)?;
        let receipt = pending
            .await
            .map_err(AppError::other)?
            .ok_or_else(|| AppError::Other("Transaction dropped from mempool".into()))?;

        Ok(format!("{:#x}", receipt.transaction_hash))
    }

    // ── Internal helpers ──────────────────────────────────────────────────────

    pub async fn get_fee_rate(&self, token_id: &str) -> Option<u64> {
        let url = format!("{}/fee-rate?token_id={}", self.clob_url, token_id);
        let resp = self.throttled_send(|| self.http.get(&url)).await.ok()?;
        if !resp.status().is_success() {
            return None;
        }
        let body: serde_json::Value = resp.json().await.ok()?;
        // Numeric fields may be JSON strings or numbers; handle both.
        let parse_u64 = |v: &serde_json::Value| -> Option<u64> {
            v.as_u64().or_else(|| v.as_str()?.parse().ok())
        };
        // py-clob-client reads `base_fee`; fall back to `fee_rate_bps` for older responses.
        parse_u64(body.get("base_fee").or_else(|| body.get("fee_rate_bps"))?)
    }

    /// Look up whether a token belongs to a neg-risk (binary) market.
    /// Returns false on any error (safe default — will just produce an invalid sig for neg-risk markets
    /// if the lookup fails, but at least non-neg-risk markets work correctly).
    pub async fn get_neg_risk(&self, token_id: &str) -> bool {
        let url = format!(
            "{}/markets?clobTokenIds={}&limit=1",
            self.gamma_url, token_id
        );
        let resp = match self.throttled_send(|| self.http.get(&url)).await {
            Ok(r) if r.status().is_success() => r,
            _ => return false,
        };
        let markets: Vec<serde_json::Value> = match resp.json().await {
            Ok(v) => v,
            _ => return false,
        };
        markets
            .first()
            .and_then(|m| m.get("negRisk"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
    }

    /// Fetch price history for a market from the Polymarket Data API.
    /// Returns a list of `(unix_timestamp, price)` pairs sorted oldest→newest.
    /// `interval` is e.g. `"1d"` or `"1w"`, `fidelity` is minutes per point (e.g. `60`).
    pub async fn get_price_history(
        &self,
        condition_id: &str,
        interval: &str,
        fidelity: u32,
    ) -> Result<Vec<PricePoint>> {
        let url = format!(
            "{}/prices-history?market={}&interval={}&fidelity={}",
            self.data_url, condition_id, interval, fidelity
        );
        let resp = self
            .throttled_send(|| self.http.get(&url))
            .await
            .map_err(net_err)?;
        if !resp.status().is_success() {
            return Err(api_err(resp).await);
        }
        let body: serde_json::Value = resp.json().await?;
        let points = body
            .get("history")
            .and_then(|h| h.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|pt| {
                        let t = pt.get("t")?.as_u64()?;
                        let p = pt.get("p")?.as_f64()?;
                        Some((t, p))
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        Ok(points)
    }

    /// Calculate the taker fee in USDC for an order using Polymarket's bell-curve formula:
    ///   fee = size × (fee_rate_bps / 10_000) × price × (1 − price)
    /// The fee peaks at p = 0.50 and is symmetric around it.
    pub fn calculate_fee(size: f64, price: f64, fee_rate_bps: u64) -> f64 {
        let rate = fee_rate_bps as f64 / 10_000.0;
        (size * rate * price * (1.0 - price) * 100_000.0).round() / 100_000.0
    }

    /// Convert a raw Gamma market into our Market type.
    /// If `fetch_books` is true, supplement prices from the CLOB order book.
    async fn gamma_to_market(&self, gm: GammaMarket, fetch_books: bool) -> Option<Market> {
        if gm.condition_id.is_empty() && gm.question.is_empty() {
            return None;
        }

        let outcomes_raw = parse_json_str_array(gm.outcomes.as_deref().unwrap_or("[]"));
        let prices_raw = parse_json_str_array(gm.outcome_prices.as_deref().unwrap_or("[]"));
        let token_ids = parse_json_str_array(gm.clob_token_ids.as_deref().unwrap_or("[]"));

        let status = if gm.closed {
            MarketStatus::Closed
        } else if gm.active {
            MarketStatus::Active
        } else {
            MarketStatus::Unknown
        };

        let volume = value_to_f64_str(&gm.volume);
        let liquidity = value_to_f64_str(&gm.liquidity);

        let tags: Vec<String> = gm
            .tags
            .unwrap_or_default()
            .into_iter()
            .map(|t| t.label)
            .collect();

        let mut outcomes = Vec::new();
        for (i, name) in outcomes_raw.iter().enumerate() {
            let gamma_price: f64 = prices_raw
                .get(i)
                .and_then(|s| s.parse().ok())
                .unwrap_or(0.5);
            let token_id = token_ids.get(i).cloned().unwrap_or_default();

            let (bid, ask, bid_depth, ask_depth) = if fetch_books && !token_id.is_empty() {
                match self.get_order_book(&token_id).await {
                    Ok(book) => {
                        let best_bid = book.bids.first().map(|l| l.price).unwrap_or(gamma_price);
                        let best_ask = book.asks.first().map(|l| l.price).unwrap_or(gamma_price);
                        let bd: f64 = book.bids.iter().map(|l| l.size).sum();
                        let ad: f64 = book.asks.iter().map(|l| l.size).sum();
                        (best_bid, best_ask, bd, ad)
                    }
                    Err(_) => (gamma_price, gamma_price, 0.0, 0.0),
                }
            } else {
                (gamma_price, gamma_price, 0.0, 0.0)
            };

            outcomes.push(Outcome {
                name: name.clone(),
                token_id,
                price: (bid + ask) / 2.0,
                bid,
                ask,
                bid_depth,
                ask_depth,
            });
        }

        let slug = if !gm.slug.is_empty() {
            gm.slug.clone()
        } else {
            gm.market_slug.clone()
        };
        let group_slug = gm.group_slug.clone();

        Some(Market {
            condition_id: gm.condition_id,
            question: gm.question,
            description: gm.description,
            slug,
            group_slug,
            status,
            end_date: gm.end_date,
            volume,
            liquidity,
            outcomes,
            category: gm.category,
            tags,
            neg_risk: gm.neg_risk,
        })
    }

    fn require_auth(&self) -> Result<(&ClobAuth, &LocalWallet)> {
        let auth = self.auth.as_ref().ok_or_else(|| {
            AppError::Auth(
                "No CLOB credentials configured (POLY_API_KEY / POLY_API_SECRET / POLY_API_PASSPHRASE)".into()
            )
        })?;
        let wallet = self.wallet.as_ref().ok_or_else(|| {
            AppError::Auth("No wallet private key configured (POLY_PRIVATE_KEY)".into())
        })?;
        Ok((auth, wallet))
    }

    /// Returns `true` when both a wallet private key and CLOB API credentials are present.
    pub fn has_credentials(&self) -> bool {
        self.wallet.is_some() && self.auth.is_some()
    }

    /// Lightweight startup credential probe.
    ///
    /// Returns `None` when credentials are absent (anonymous mode — not an error) or valid.
    /// Returns `Some(message)` when credentials are present but the API rejects them, so the
    /// TUI can show a persistent warning before the user tries to place an order.
    pub async fn check_credentials(&self) -> Option<String> {
        if !self.has_credentials() {
            return None; // no credentials configured — anonymous mode is fine
        }
        match self.get_open_orders().await {
            Ok(_) => None, // accepted — credentials are valid
            Err(AppError::Auth(msg)) => Some(msg),
            Err(AppError::Api { status: 401, .. }) => {
                Some("API key rejected (401 Unauthorized) — check POLY_API_KEY".into())
            }
            Err(AppError::Api { status: 403, .. }) => {
                Some("API key forbidden (403) — key may be expired or for wrong environment".into())
            }
            Err(_) => None, // network / transient error — don't treat as bad credentials
        }
    }

    /// Return the EOA address string (from POLY_MARKET_KEY / POLY_PRIVATE_KEY).
    pub fn wallet_address_str(&self) -> String {
        self.wallet
            .as_ref()
            .map(|w| format!("{:#x}", w.address()))
            .unwrap_or_else(|| "<no wallet>".to_string())
    }

    /// Derive CLOB API credentials by signing a ClobAuth EIP-712 message with
    /// the configured private key and POSTing to `/auth/api-key`.
    ///
    /// Returns `(api_key, secret, passphrase)` which should be saved to .env as
    /// POLY_API_KEY, POLY_API_SECRET, POLY_API_PASSPHRASE.
    pub async fn derive_api_creds(&self) -> Result<(String, String, String)> {
        let wallet = self.wallet.as_ref().ok_or_else(|| {
            AppError::Auth("No wallet configured (POLY_PRIVATE_KEY / POLY_MARKET_KEY)".into())
        })?;

        let address = wallet.address();

        // Fetch server timestamp — CLOB rejects requests with large clock skew.
        // The /time endpoint returns a plain integer string (e.g. "1775295881").
        let timestamp: String = {
            let t_resp = self
                .http
                .get(format!("{}/time", self.clob_url))
                .send()
                .await
                .map_err(net_err)?;
            let body = t_resp.text().await.unwrap_or_default();
            let trimmed = body.trim().trim_matches('"');
            // Accept plain integer or {"time": N} or {"timestamp": N}
            if trimmed.chars().all(|c| c.is_ascii_digit()) {
                trimmed.to_string()
            } else if let Ok(v) = serde_json::from_str::<serde_json::Value>(&body) {
                v.get("time")
                    .or_else(|| v.get("timestamp"))
                    .and_then(|x| x.as_f64())
                    .map(|f| (f as u64).to_string())
                    .unwrap_or_else(|| {
                        std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .expect("system clock before Unix epoch")
                            .as_secs()
                            .to_string()
                    })
            } else {
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .expect("system clock before Unix epoch")
                    .as_secs()
                    .to_string()
            }
        };

        // ── EIP-712 ClobAuth signing ──────────────────────────────────────────
        // Domain has no verifyingContract (3-field variant)
        let domain_type_hash =
            ethers::utils::keccak256(b"EIP712Domain(string name,string version,uint256 chainId)");
        let domain_name_hash = ethers::utils::keccak256(b"ClobAuthDomain");
        let domain_version_hash = ethers::utils::keccak256(b"1");

        let domain_separator = {
            let mut enc = Vec::with_capacity(4 * 32);
            enc.extend_from_slice(&domain_type_hash);
            enc.extend_from_slice(&domain_name_hash);
            enc.extend_from_slice(&domain_version_hash);
            let mut v = [0u8; 32];
            U256::from(137u64).to_big_endian(&mut v);
            enc.extend_from_slice(&v);
            ethers::utils::keccak256(enc)
        };

        let type_hash = ethers::utils::keccak256(
            b"ClobAuth(address address,string timestamp,uint256 nonce,string message)",
        );

        let message = "This message attests that I control the given wallet";
        let timestamp_hash = ethers::utils::keccak256(timestamp.as_bytes());
        let message_hash = ethers::utils::keccak256(message.as_bytes());

        let nonce_val: u64 = if self.funder_address.is_some() { 2 } else { 0 };
        let struct_hash = {
            let mut enc = Vec::with_capacity(5 * 32);
            enc.extend_from_slice(&type_hash);
            let mut v = [0u8; 32];
            v[12..].copy_from_slice(address.as_bytes());
            enc.extend_from_slice(&v);
            enc.extend_from_slice(&timestamp_hash);
            let mut v = [0u8; 32];
            U256::from(nonce_val).to_big_endian(&mut v);
            enc.extend_from_slice(&v);
            enc.extend_from_slice(&message_hash);
            ethers::utils::keccak256(enc)
        };

        let digest = {
            let mut msg = [0u8; 66];
            msg[0] = 0x19;
            msg[1] = 0x01;
            msg[2..34].copy_from_slice(&domain_separator);
            msg[34..66].copy_from_slice(&struct_hash);
            ethers::utils::keccak256(msg)
        };

        let signature = wallet.sign_hash(digest.into()).map_err(AppError::other)?;
        let sig_str = format!("0x{}", signature);
        let addr_str = format!("{:#x}", address);

        // ── POST /auth/api-key ────────────────────────────────────────────────
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert("POLY_ADDRESS", addr_str.parse().map_err(AppError::other)?);
        headers.insert("POLY_SIGNATURE", sig_str.parse().map_err(AppError::other)?);
        headers.insert(
            "POLY_TIMESTAMP",
            timestamp.parse().map_err(AppError::other)?,
        );
        headers.insert(
            "POLY_NONCE",
            nonce_val.to_string().parse().map_err(AppError::other)?,
        );

        let clob_url = self.clob_url.clone();
        let resp = self
            .throttled_send(|| {
                self.http
                    .post(format!("{}/auth/api-key", clob_url))
                    .headers(headers.clone())
            })
            .await
            .map_err(net_err)?;

        if !resp.status().is_success() {
            return Err(api_err(resp).await);
        }

        #[derive(Deserialize)]
        struct RawCreds {
            #[serde(rename = "apiKey")]
            api_key: String,
            secret: String,
            passphrase: String,
        }

        let creds: RawCreds = resp.json().await?;
        Ok((creds.api_key, creds.secret, creds.passphrase))
    }

    /// Fetch the YES-token price from the CLOB `prices-history` API at
    /// `hours_before` hours before `end_date_unix` (Unix seconds).
    ///
    /// Returns the closing price of the latest hourly candle whose timestamp
    /// is ≤ `end_date_unix − hours_before × 3600`.  Returns `Ok(None)` when
    /// no data is available for the requested window.
    pub async fn get_calibration_price(
        &self,
        token_id: &str,
        end_date_unix: i64,
        hours_before: u64,
    ) -> Result<Option<f64>> {
        #[derive(Deserialize)]
        struct PricesHistory {
            history: Vec<PricePoint>,
        }
        #[derive(Deserialize)]
        struct PricePoint {
            t: i64,
            p: f64,
        }

        let url = format!(
            "{}/prices-history?market={}&interval=all&fidelity=60",
            self.clob_url, token_id,
        );
        let resp = self
            .throttled_send(|| self.http.get(&url))
            .await
            .map_err(net_err)?;
        if !resp.status().is_success() {
            return Ok(None);
        }
        let body: PricesHistory = resp.json().await?;
        let target_ts = end_date_unix - (hours_before * 3600) as i64;
        let price = body
            .history
            .iter()
            .rev()
            .find(|pt| pt.t <= target_ts)
            .map(|pt| pt.p);
        Ok(price)
    }
}

// ── Utility functions ─────────────────────────────────────────────────────────

fn parse_json_str_array(s: &str) -> Vec<String> {
    serde_json::from_str::<Vec<String>>(s).unwrap_or_default()
}

/// Extract the Yes-outcome CLOB token ID (first element) from the
/// JSON-encoded `clobTokenIds` array, e.g. `"[\"12345\", \"67890\"]"`.
fn parse_yes_token_id(clob_token_ids: Option<&str>) -> Option<String> {
    let ids = parse_json_str_array(clob_token_ids?);
    ids.into_iter().next().filter(|s| !s.is_empty())
}

/// Derive the resolution outcome from `outcomePrices`.
/// The Gamma API encodes both as JSON-string arrays, e.g. `"[\"Yes\", \"No\"]"`.
/// The winning outcome is the one whose settlement price is ≈ 1.0.
/// Returns `None` if no clear winner is found (market unresolved or bad data).
fn derive_resolution(outcomes_json: Option<&str>, prices_json: Option<&str>) -> Option<String> {
    let outcomes = parse_json_str_array(outcomes_json.unwrap_or("[]"));
    let prices = parse_json_str_array(prices_json.unwrap_or("[]"));
    if outcomes.is_empty() || outcomes.len() != prices.len() {
        return None;
    }
    outcomes
        .into_iter()
        .zip(prices.iter())
        .find_map(|(name, price_str)| {
            let p: f64 = price_str.parse().ok()?;
            if (p - 1.0).abs() < 0.01 {
                Some(name)
            } else {
                None
            }
        })
}

fn value_to_f64_str(v: &Option<serde_json::Value>) -> f64 {
    match v {
        Some(serde_json::Value::Number(n)) => n.as_f64().unwrap_or(0.0),
        Some(serde_json::Value::String(s)) => s.parse().unwrap_or(0.0),
        _ => 0.0,
    }
}

fn parse_value_f64(v: &Option<serde_json::Value>) -> Option<f64> {
    match v {
        Some(serde_json::Value::Number(n)) => n.as_f64(),
        Some(serde_json::Value::String(s)) => s.parse().ok(),
        _ => None,
    }
}

fn parse_order_status(s: Option<&str>) -> OrderStatus {
    match s {
        Some("LIVE" | "OPEN") => OrderStatus::Live,
        Some("MATCHED" | "FILLED") => OrderStatus::Filled,
        Some("CANCELED" | "CANCELLED") => OrderStatus::Cancelled,
        _ => OrderStatus::Unknown,
    }
}

fn urlencoded(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            ' ' => '+',
            c if c.is_alphanumeric() || c == '-' || c == '_' || c == '.' => c,
            _ => c, // reqwest will handle further encoding if needed
        })
        .collect()
}
