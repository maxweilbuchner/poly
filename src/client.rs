use ethers::providers::{Http, Provider};
use ethers::signers::{LocalWallet, Signer};
use ethers::types::{H160, U256};
use rand::Rng;
use reqwest::Client;
use serde::Deserialize;
use std::str::FromStr;

use crate::auth::ClobAuth;
use crate::types::{Market, MarketStatus, Order, OrderBook, OrderStatus, OrderType, Outcome,
                   Position, PriceLevel, Side};

pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

const GAMMA_API: &str = "https://gamma-api.polymarket.com";
const CLOB_API: &str = "https://clob.polymarket.com";
const CTF_EXCHANGE: &str = "0x4bFb41d5B3570DeFd03C39a9A4D8dE6Bd8B8982E";
const USDC_ADDRESS: &str = "0x2791Bca1f2de4661ED88A30C99A7a9449Aa84174";

ethers::contract::abigen!(
    ERC20,
    r#"[
        function balanceOf(address account) external view returns (uint256)
        function allowance(address owner, address spender) external view returns (uint256)
    ]"#
);

// ── Raw Gamma API types ───────────────────────────────────────────────────────

#[derive(Deserialize)]
struct GammaMarket {
    #[serde(rename = "conditionId", default)]
    condition_id: String,
    #[serde(default)]
    question: String,
    #[serde(rename = "marketSlug", default)]
    market_slug: String,
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
    #[serde(rename = "category", default)]
    category: Option<String>,
    #[serde(default)]
    tags: Option<Vec<TagEntry>>,
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

#[derive(Clone)]
pub struct PolyClient {
    http: Client,
    wallet: Option<LocalWallet>,
    funder_address: Option<H160>,
    pub auth: Option<ClobAuth>,
    provider: Option<Provider<Http>>,
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
        Self { http: Client::new(), wallet, funder_address, auth, provider }
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
        let active_param = if active_only { "&active=true&closed=false" } else { "" };
        // Fetch a generous over-fetch so client-side filtering has enough to work with
        let fetch_limit = (limit * 8).max(100);
        let url = format!(
            "{}/markets?search={}&limit={}{}",
            GAMMA_API,
            urlencoded(query),
            fetch_limit,
            active_param
        );

        let resp = self.http.get(&url).send().await?;
        if !resp.status().is_success() {
            return Err(format!("Gamma search failed: {}", resp.status()).into());
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
    pub async fn get_top_markets(&self, limit: usize, category: Option<&str>) -> Result<Vec<Market>> {
        // Over-fetch so category filtering still yields `limit` results.
        let fetch_limit = if category.is_some() { (limit * 4).max(100) } else { limit };
        let url = format!(
            "{}/markets?active=true&closed=false&order=volume&ascending=false&limit={}",
            GAMMA_API, fetch_limit
        );

        let resp = self.http.get(&url).send().await?;
        if !resp.status().is_success() {
            return Err(format!("Gamma /markets failed: {}", resp.status()).into());
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

    /// Fetch a single market by its condition ID (hex string).
    pub async fn get_market_by_id(&self, condition_id: &str) -> Result<Option<Market>> {
        let url = format!("{}/markets/{}", GAMMA_API, condition_id);
        let resp = self.http.get(&url).send().await?;
        if resp.status().as_u16() == 404 {
            return Ok(None);
        }
        if !resp.status().is_success() {
            return Err(format!("Gamma markets/{} failed: {}", condition_id, resp.status()).into());
        }
        let gm: GammaMarket = resp.json().await?;
        Ok(self.gamma_to_market(gm, true).await)
    }

    /// Fetch a market (or markets within an event) by slug.
    /// Tries the /events endpoint first (event slug), then /markets directly.
    pub async fn get_market_by_slug(&self, slug: &str) -> Result<Vec<Market>> {
        // Try as an event slug first
        let url = format!("{}/events?slug={}", GAMMA_API, urlencoded(slug));
        let resp = self.http.get(&url).send().await?;
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
        let url2 = format!("{}/markets?slug={}", GAMMA_API, urlencoded(slug));
        let resp2 = self.http.get(&url2).send().await?;
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
        let url = format!("{}/book?token_id={}", CLOB_API, token_id);
        let resp = self.http.get(&url).send().await?;
        if !resp.status().is_success() {
            return Err(format!("CLOB /book failed: {}", resp.status()).into());
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
            levels.sort_by(|a, b| b.price.partial_cmp(&a.price).unwrap());
            levels
        };

        let mut bids = parse_levels(book.bids);
        let mut asks = parse_levels(book.asks);
        // bids: highest first, asks: lowest first
        bids.sort_by(|a, b| b.price.partial_cmp(&a.price).unwrap());
        asks.sort_by(|a, b| a.price.partial_cmp(&b.price).unwrap());

        Ok(OrderBook { token_id: token_id.to_string(), bids, asks })
    }

    // ── Account info ──────────────────────────────────────────────────────────

    /// USDC balance on-chain (Polygon mainnet).
    pub async fn get_balance(&self) -> Result<f64> {
        let provider = self.provider.as_ref().ok_or("No RPC URL configured")?;
        let address = self
            .funder_address
            .or_else(|| self.wallet.as_ref().map(|w| w.address()))
            .ok_or("No wallet or funder address configured")?;

        let usdc: H160 = USDC_ADDRESS.parse()?;
        let contract = ERC20::new(usdc, provider.clone().into());
        let raw: U256 = contract.balance_of(address).call().await?;
        Ok(raw.to_string().parse::<f64>().unwrap_or(0.0) / 1_000_000.0)
    }

    /// USDC allowance granted to the CTF Exchange.
    pub async fn get_allowance(&self) -> Result<f64> {
        let provider = self.provider.as_ref().ok_or("No RPC URL configured")?;
        let address = self
            .funder_address
            .or_else(|| self.wallet.as_ref().map(|w| w.address()))
            .ok_or("No wallet or funder address configured")?;

        let usdc: H160 = USDC_ADDRESS.parse()?;
        let spender: H160 = CTF_EXCHANGE.parse()?;
        let contract = ERC20::new(usdc, provider.clone().into());
        let raw: U256 = contract.allowance(address, spender).call().await?;
        Ok(raw.to_string().parse::<f64>().unwrap_or(0.0) / 1_000_000.0)
    }

    // ── Orders ────────────────────────────────────────────────────────────────

    /// List open orders for the authenticated user.
    pub async fn get_open_orders(&self) -> Result<Vec<Order>> {
        let (auth, wallet) = self.require_auth()?;
        let path = "/data/orders?status=live";
        let url = format!("{}{}", CLOB_API, path);
        let signer_str = format!("{:#x}", wallet.address());
        let headers = auth.headers("GET", path, None, &signer_str)?;

        let resp = self.http.get(&url).headers(headers).send().await?;
        if !resp.status().is_success() {
            return Err(format!("CLOB /data/orders failed: {}", resp.status()).into());
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

        let raw: Vec<RawOrder> = resp.json().await?;
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

    /// Get a single order by ID (raw JSON for inspection).
    #[allow(dead_code)]
    pub async fn get_order(&self, order_id: &str) -> Result<serde_json::Value> {
        let (auth, wallet) = self.require_auth()?;
        let path = format!("/data/order/{}", order_id);
        let url = format!("{}{}", CLOB_API, path);
        let signer_str = format!("{:#x}", wallet.address());
        let headers = auth.headers("GET", &path, None, &signer_str)?;

        let resp = self.http.get(&url).headers(headers).send().await?;
        if !resp.status().is_success() {
            return Err(format!("CLOB /data/order/{} failed: {}", order_id, resp.status()).into());
        }
        Ok(resp.json().await?)
    }

    // ── Positions ─────────────────────────────────────────────────────────────

    /// List current positions for the authenticated user.
    pub async fn get_positions(&self) -> Result<Vec<Position>> {
        let (auth, wallet) = self.require_auth()?;
        let signer_str = format!("{:#x}", wallet.address());
        let address = self.funder_address.unwrap_or(wallet.address());
        let address_str = format!("{:#x}", address);

        let path = format!("/data/positions?user={}&sizeThreshold=0.1", address_str);
        let url = format!("{}{}", CLOB_API, path);
        let headers = auth.headers("GET", &path, None, &signer_str)?;

        let resp = self.http.get(&url).headers(headers).send().await?;
        if !resp.status().is_success() {
            return Err(format!("CLOB /data/positions failed: {}", resp.status()).into());
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
            #[serde(rename = "currentPrice", default)]
            current_price: Option<serde_json::Value>,
            #[serde(rename = "realizedPnl", default)]
            realized_pnl: Option<serde_json::Value>,
            #[serde(rename = "unrealizedPnl", default)]
            unrealized_pnl: Option<serde_json::Value>,
        }

        let raw: Vec<RawPosition> = resp.json().await?;
        Ok(raw
            .into_iter()
            .map(|p| Position {
                market_id: p.condition_id,
                market_question: p.title.unwrap_or_default(),
                outcome: p.outcome.unwrap_or_default(),
                token_id: p.asset,
                size: parse_value_f64(&p.size).unwrap_or(0.0),
                avg_price: parse_value_f64(&p.avg_price).unwrap_or(0.0),
                current_price: parse_value_f64(&p.current_price).unwrap_or(0.0),
                realized_pnl: parse_value_f64(&p.realized_pnl).unwrap_or(0.0),
                unrealized_pnl: parse_value_f64(&p.unrealized_pnl).unwrap_or(0.0),
            })
            .collect())
    }

    // ── Order placement ───────────────────────────────────────────────────────

    /// Place a limit order on the CLOB.
    ///
    /// `token_id` is the CLOB token ID for the specific outcome.
    /// `price` is in [0.01, 0.99].
    /// `size` is the number of shares (min 5, min $1 USDC).
    /// Returns the CLOB `orderID` on success.
    pub async fn place_order(
        &self,
        token_id: &str,
        price: f64,
        size: f64,
        side: Side,
        order_type: OrderType,
    ) -> Result<String> {
        let fee_rate_bps = self.get_fee_rate(token_id).await.unwrap_or(0);
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

        // Round to CLOB-acceptable precision: size to 2dp, cost to 4dp
        let rounded_size = (size * 100.0).round() / 100.0;
        let rounded_cost = (rounded_size * price * 10_000.0).round() / 10_000.0;

        let decimals = 1_000_000_f64;
        let size_scaled = (rounded_size * decimals).round() as u64;
        let cost_scaled = (rounded_cost * decimals).round() as u64;

        let (maker_amount, taker_amount, side_u8) = match side {
            Side::Buy => (cost_scaled, size_scaled, 0u8),
            Side::Sell => (size_scaled, cost_scaled, 1u8),
        };

        let signature_type: u8 =
            if self.funder_address.is_some() && self.funder_address != Some(signer) { 1 } else { 0 };

        // ── EIP-712 signing ───────────────────────────────────────────────────
        // Type hash uses exact Polymarket field names (camelCase)
        let type_hash = ethers::utils::keccak256(
            b"Order(uint256 salt,address maker,address signer,address taker,uint256 tokenId,uint256 makerAmount,uint256 takerAmount,uint256 expiration,uint256 nonce,uint256 feeRateBps,uint8 side,uint8 signatureType)",
        );

        let domain_type_hash = ethers::utils::keccak256(
            b"EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)",
        );
        let domain_name_hash = ethers::utils::keccak256(b"Polymarket CTF Exchange");
        let domain_version_hash = ethers::utils::keccak256(b"1");
        let chain_id = U256::from(137u64);
        let verifying_contract: H160 = CTF_EXCHANGE.parse()?;

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
            let mut v = [0u8; 32]; salt.to_big_endian(&mut v); enc.extend_from_slice(&v);
            let mut v = [0u8; 32]; v[12..].copy_from_slice(maker.as_bytes()); enc.extend_from_slice(&v);
            let mut v = [0u8; 32]; v[12..].copy_from_slice(signer.as_bytes()); enc.extend_from_slice(&v);
            let mut v = [0u8; 32]; v[12..].copy_from_slice(taker.as_bytes()); enc.extend_from_slice(&v);
            let mut v = [0u8; 32]; token_id_u256.to_big_endian(&mut v); enc.extend_from_slice(&v);
            let mut v = [0u8; 32]; U256::from(maker_amount).to_big_endian(&mut v); enc.extend_from_slice(&v);
            let mut v = [0u8; 32]; U256::from(taker_amount).to_big_endian(&mut v); enc.extend_from_slice(&v);
            enc.extend_from_slice(&[0u8; 32]); // expiration = 0
            enc.extend_from_slice(&[0u8; 32]); // nonce = 0
            let mut v = [0u8; 32]; U256::from(fee_rate_bps).to_big_endian(&mut v); enc.extend_from_slice(&v);
            let mut v = [0u8; 32]; v[31] = side_u8; enc.extend_from_slice(&v);
            let mut v = [0u8; 32]; v[31] = signature_type; enc.extend_from_slice(&v);
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

        let signature = wallet.sign_hash(digest.into())?;
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
                "expiration": "0",
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
        let signer_str = format!("{:#x}", signer);
        let mut headers = auth.headers("POST", "/order", Some(&body_str), &signer_str)?;
        headers.insert("Content-Type", "application/json".parse()?);

        let resp = self
            .http
            .post(format!("{}/order", CLOB_API))
            .headers(headers)
            .body(body_str)
            .send()
            .await?;

        if !resp.status().is_success() {
            let text = resp.text().await?;
            return Err(format!("Order placement failed: {}", text).into());
        }

        let json: serde_json::Value = resp.json().await?;
        let order_id = json
            .get("orderID")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        Ok(order_id)
    }

    /// Cancel a single open order by its CLOB order ID.
    pub async fn cancel_order(&self, order_id: &str) -> Result<()> {
        let (auth, wallet) = self.require_auth()?;
        let body = serde_json::json!({ "orderID": order_id });
        let body_str = body.to_string();
        let signer_str = format!("{:#x}", wallet.address());
        let mut headers = auth.headers("DELETE", "/order", Some(&body_str), &signer_str)?;
        headers.insert("Content-Type", "application/json".parse()?);

        let resp = self
            .http
            .delete(format!("{}/order", CLOB_API))
            .headers(headers)
            .body(body_str)
            .send()
            .await?;

        if !resp.status().is_success() {
            let text = resp.text().await?;
            return Err(format!("Cancel failed: {}", text).into());
        }
        Ok(())
    }

    /// Cancel all open orders for the authenticated user.
    pub async fn cancel_all_orders(&self) -> Result<()> {
        let (auth, wallet) = self.require_auth()?;
        let signer_str = format!("{:#x}", wallet.address());
        let headers = auth.headers("DELETE", "/orders", None, &signer_str)?;

        let resp = self
            .http
            .delete(format!("{}/orders", CLOB_API))
            .headers(headers)
            .send()
            .await?;

        if !resp.status().is_success() {
            let text = resp.text().await?;
            return Err(format!("Cancel-all failed: {}", text).into());
        }
        Ok(())
    }

    // ── Internal helpers ──────────────────────────────────────────────────────

    async fn get_fee_rate(&self, token_id: &str) -> Option<u64> {
        let url = format!("{}/fee-rate?token_id={}", CLOB_API, token_id);
        let resp = self.http.get(&url).send().await.ok()?;
        if !resp.status().is_success() {
            return None;
        }
        let body: serde_json::Value = resp.json().await.ok()?;
        body.get("minimum_order_size")
            .and_then(|_| body.get("base_fee"))
            .and_then(|v| v.as_u64())
            .or_else(|| body.get("fee_rate_bps").and_then(|v| v.as_u64()))
    }

    /// Convert a raw Gamma market into our Market type.
    /// If `fetch_books` is true, supplement prices from the CLOB order book.
    async fn gamma_to_market(&self, gm: GammaMarket, fetch_books: bool) -> Option<Market> {
        if gm.condition_id.is_empty() && gm.question.is_empty() {
            return None;
        }

        let outcomes_raw = parse_json_str_array(gm.outcomes.as_deref().unwrap_or("[]"));
        let prices_raw =
            parse_json_str_array(gm.outcome_prices.as_deref().unwrap_or("[]"));
        let token_ids =
            parse_json_str_array(gm.clob_token_ids.as_deref().unwrap_or("[]"));

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
            let gamma_price: f64 = prices_raw.get(i).and_then(|s| s.parse().ok()).unwrap_or(0.5);
            let token_id = token_ids.get(i).cloned().unwrap_or_default();

            let (bid, ask, bid_depth, ask_depth) = if fetch_books && !token_id.is_empty() {
                match self.get_order_book(&token_id).await {
                    Ok(book) => {
                        let best_bid =
                            book.bids.first().map(|l| l.price).unwrap_or(gamma_price);
                        let best_ask =
                            book.asks.first().map(|l| l.price).unwrap_or(gamma_price);
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

        let slug = gm.market_slug.clone();

        Some(Market {
            condition_id: gm.condition_id,
            question: gm.question,
            slug,
            status,
            end_date: gm.end_date,
            volume,
            liquidity,
            outcomes,
            category: gm.category,
            tags,
        })
    }

    fn require_auth(
        &self,
    ) -> std::result::Result<(&ClobAuth, &LocalWallet), Box<dyn std::error::Error + Send + Sync>>
    {
        let auth = self.auth.as_ref().ok_or(
            "No CLOB credentials. Set POLY_API_KEY, POLY_API_SECRET, POLY_API_PASSPHRASE in .env",
        )?;
        let wallet = self.wallet.as_ref().ok_or(
            "No wallet key. Set POLY_PRIVATE_KEY in .env",
        )?;
        Ok((auth, wallet))
    }
}

// ── Utility functions ─────────────────────────────────────────────────────────

fn parse_json_str_array(s: &str) -> Vec<String> {
    serde_json::from_str::<Vec<String>>(s).unwrap_or_default()
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
