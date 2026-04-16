use std::fmt;

/// Typed application errors with actionable Display messages.
#[derive(Debug)]
pub enum AppError {
    /// Missing or invalid credentials / config
    Auth(String),
    /// Network-level failure (connect refused, timeout, DNS)
    Network(String),
    /// API returned a non-2xx response
    Api { status: u16, message: String },
    /// Anything else
    Other(Box<dyn std::error::Error + Send + Sync>),
}

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AppError::Auth(msg) => write!(
                f,
                "{}\n  Hint: run `poly setup` to configure credentials",
                msg
            ),
            AppError::Network(msg) => write!(
                f,
                "Network error — {}\n  Hint: check your internet connection or POLYGON_RPC_URL",
                msg
            ),
            AppError::Api { status, message } => {
                write!(f, "API error (HTTP {}): {}", status, message)
            }
            AppError::Other(e) => fmt::Display::fmt(e, f),
        }
    }
}

impl std::error::Error for AppError {}

impl From<reqwest::Error> for AppError {
    fn from(e: reqwest::Error) -> Self {
        AppError::from_reqwest(e)
    }
}

impl From<String> for AppError {
    fn from(s: String) -> Self {
        AppError::Other(s.into())
    }
}

impl From<&str> for AppError {
    fn from(s: &str) -> Self {
        AppError::Other(s.to_string().into())
    }
}

impl From<Box<dyn std::error::Error + Send + Sync>> for AppError {
    fn from(e: Box<dyn std::error::Error + Send + Sync>) -> Self {
        AppError::Other(e)
    }
}

impl From<std::io::Error> for AppError {
    fn from(e: std::io::Error) -> Self {
        AppError::Other(Box::new(e))
    }
}

impl AppError {
    /// Returns `true` if this is a credentials / auth error.
    pub fn is_auth(&self) -> bool {
        matches!(self, AppError::Auth(_))
    }

    /// Wrap any boxable error as `Other`.
    pub fn other<E: std::error::Error + Send + Sync + 'static>(e: E) -> Self {
        AppError::Other(Box::new(e))
    }

    /// Convert a reqwest transport error into the appropriate variant.
    pub fn from_reqwest(e: reqwest::Error) -> Self {
        let url = e.url().map(|u| u.as_str()).unwrap_or("unknown URL");
        if e.is_timeout() {
            AppError::Network(format!("request timed out ({})", url))
        } else if e.is_connect() {
            AppError::Network(format!("connection refused or DNS failure ({})", url))
        } else {
            AppError::Other(e.into())
        }
    }

    /// Build an Api error from an HTTP status code and raw response body.
    /// Tries to extract an inner message from `{"error":"..."}` or `{"message":"..."}`.
    pub fn from_api_body(status: u16, body: &str) -> Self {
        let message = serde_json::from_str::<serde_json::Value>(body)
            .ok()
            .and_then(|v| {
                v.get("error")
                    .or_else(|| v.get("message"))
                    .or_else(|| v.get("errorMsg"))  // Polymarket CLOB uses errorMsg
                    .and_then(|m| m.as_str())
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_string())
            })
            .unwrap_or_else(|| {
                let t = body.trim();
                if t.is_empty() {
                    format!("HTTP {}", status)
                } else {
                    t.to_string()
                }
            });
        AppError::Api { status, message }
    }
}
