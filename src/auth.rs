use base64::{
    engine::general_purpose::{STANDARD, URL_SAFE},
    Engine as _,
};
use hmac::{Hmac, Mac};
use reqwest::header::HeaderMap;
use sha2::Sha256;
use std::time::{SystemTime, UNIX_EPOCH};

type HmacSha256 = Hmac<Sha256>;

#[derive(Clone)]
pub struct ClobAuth {
    pub key: String,
    pub secret: String,
    pub passphrase: String,
}

impl ClobAuth {
    pub fn new(key: String, secret: String, passphrase: String) -> Self {
        Self { key, secret, passphrase }
    }

    /// Build HMAC-SHA256 authentication headers for CLOB REST requests.
    ///
    /// message = timestamp + method + path + body
    /// signature = url_safe_base64(hmac_sha256(url_safe_base64_decode(secret), message))
    pub fn headers(
        &self,
        method: &str,
        path: &str,
        body: Option<&str>,
        address: &str,
    ) -> Result<HeaderMap, Box<dyn std::error::Error + Send + Sync>> {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)?
            .as_secs()
            .to_string();

        let message = format!("{}{}{}{}", timestamp, method, path, body.unwrap_or(""));

        let decoded_secret = URL_SAFE
            .decode(&self.secret)
            .or_else(|_| STANDARD.decode(&self.secret))?;
        let mut mac = HmacSha256::new_from_slice(&decoded_secret)?;
        mac.update(message.as_bytes());
        let signature = URL_SAFE.encode(mac.finalize().into_bytes());

        let mut headers = HeaderMap::new();
        headers.insert("POLY_ADDRESS", address.parse()?);
        headers.insert("POLY_API_KEY", self.key.parse()?);
        headers.insert("POLY_PASSPHRASE", self.passphrase.parse()?);
        headers.insert("POLY_TIMESTAMP", timestamp.parse()?);
        headers.insert("POLY_SIGNATURE", signature.parse()?);

        Ok(headers)
    }

    /// Auth message for the user WebSocket channel.
    #[allow(dead_code)]
    pub fn ws_auth_message(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "user",
            "auth": {
                "apiKey":     self.key,
                "secret":     self.secret,
                "passphrase": self.passphrase,
            }
        })
    }
}
