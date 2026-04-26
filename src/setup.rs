use std::io::{self, BufRead, Write};
use std::path::PathBuf;

pub async fn run() -> crate::client::Result<()> {
    println!();
    println!("  Welcome to poly — the Polymarket CLI.");
    println!("  This wizard will set up your trading credentials.");
    println!();

    let config_path = config_write_path();
    println!("  Config will be saved to: {}", config_path.display());
    println!();

    let existing = load_existing(&config_path);

    // ── Step 1: Private Key ──────────────────────────────────────────
    println!("  ── Step 1/4: Wallet Private Key ────────────────────────────");
    println!();
    println!("  Your Ethereum private key signs orders on Polymarket.");
    println!("  Export from MetaMask: Account menu -> Account details -> Show private key.");
    println!("  Format: 0x followed by 64 hex characters.");
    println!();

    let private_key = if let Some(ref pk) = existing.private_key {
        println!("  Already configured: {}", mask_secret(pk));
        if prompt_yes_no("  Keep existing? [Y/n] ", true)? {
            pk.clone()
        } else {
            read_private_key_loop()?
        }
    } else {
        read_private_key_loop()?
    };
    println!();

    // ── Step 2: CLOB API Keys ────────────────────────────────────────
    println!("  ── Step 2/4: CLOB API Credentials ──────────────────────────");
    println!();
    println!("  Required for trading, viewing orders, and positions.");
    println!("  Generate them by running: poly derive-keys");
    println!("  Or find them in your Polymarket account API settings.");
    println!();

    let (api_key, api_secret, api_passphrase) = match (
        &existing.api_key,
        &existing.api_secret,
        &existing.api_passphrase,
    ) {
        (Some(k), Some(s), Some(p)) => {
            println!("  Already configured: API key {}", mask_secret(k));
            if prompt_yes_no("  Keep existing? [Y/n] ", true)? {
                (k.clone(), s.clone(), p.clone())
            } else {
                read_clob_keys_loop()?
            }
        }
        _ => read_clob_keys_loop()?,
    };
    println!();

    // ── Step 3: RPC URL ──────────────────────────────────────────────
    println!("  ── Step 3/4: Polygon RPC URL (optional) ────────────────────");
    println!();
    println!("  Needed only for `poly balance` (on-chain USDC balance check).");
    println!("  Get a free key at https://alchemy.com or https://infura.io");
    println!("  Example: https://polygon-mainnet.g.alchemy.com/v2/YOUR_KEY");
    println!();

    let rpc_url = if let Some(ref rpc) = existing.rpc_url {
        println!("  Current: {}", truncate(rpc, 60));
        if prompt_yes_no("  Keep existing? [Y/n] ", true)? {
            Some(rpc.clone())
        } else {
            read_optional_url("  RPC URL (Enter to skip): ")?
        }
    } else {
        read_optional_url("  RPC URL (Enter to skip): ")?
    };
    println!();

    // ── Step 4: Funder Address ───────────────────────────────────────
    println!("  ── Step 4/4: Funder / Proxy Address (optional) ─────────────");
    println!();
    println!("  Only needed if you trade through a proxy wallet or Gnosis Safe.");
    println!("  Most users should skip this.");
    println!();

    let funder_address = if let Some(ref addr) = existing.funder_address {
        println!("  Current: {}", addr);
        if prompt_yes_no("  Keep existing? [Y/n] ", true)? {
            Some(addr.clone())
        } else {
            read_optional_address("  Funder address (Enter to skip): ")?
        }
    } else {
        read_optional_address("  Funder address (Enter to skip): ")?
    };
    println!();

    // ── Write config ─────────────────────────────────────────────────
    write_config(
        &config_path,
        &private_key,
        &api_key,
        &api_secret,
        &api_passphrase,
        rpc_url.as_deref(),
        funder_address.as_deref(),
        &existing.tui_section,
    )?;

    println!("  Configuration saved to {}", config_path.display());
    println!();
    println!("  Get started:");
    println!("    poly search \"Trump\"    — search markets");
    println!("    poly positions         — view your positions");
    println!("    poly                   — open the TUI dashboard");
    println!();

    Ok(())
}

// ── Input helpers ────────────────────────────────────────────────────────────

fn prompt_line(msg: &str) -> io::Result<String> {
    let mut stdout = io::stdout();
    print!("{}", msg);
    stdout.flush()?;
    let mut line = String::new();
    io::stdin().lock().read_line(&mut line)?;
    Ok(line.trim().to_string())
}

fn prompt_yes_no(msg: &str, default_yes: bool) -> io::Result<bool> {
    let input = prompt_line(msg)?;
    Ok(if input.is_empty() {
        default_yes
    } else {
        input.eq_ignore_ascii_case("y") || input.eq_ignore_ascii_case("yes")
    })
}

/// Validate (and normalize) a wallet private key.
/// Accepts `0x` + 64 hex chars, or bare 64 hex chars (auto-prefixed).
/// Returns the normalized `0x...` form on success or an error message.
pub fn validate_private_key(input: &str) -> std::result::Result<String, String> {
    let key = input.trim();
    if key.is_empty() {
        return Err("Private key cannot be empty.".into());
    }

    if key.len() == 64 && key.chars().all(|c| c.is_ascii_hexdigit()) {
        return Ok(format!("0x{}", key));
    }

    if !key.starts_with("0x") {
        return Err("Private key must start with 0x.".into());
    }

    let hex_part = &key[2..];
    if hex_part.len() != 64 {
        return Err(format!(
            "Expected 64 hex characters after 0x, got {}.",
            hex_part.len()
        ));
    }

    if !hex_part.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err("Key contains non-hex characters.".into());
    }

    Ok(key.to_string())
}

/// Repeatedly prompt for a private key until a valid one is entered.
fn read_private_key_loop() -> io::Result<String> {
    loop {
        let key = rpassword::prompt_password("  Private key (input hidden): ")?;
        match validate_private_key(&key) {
            Ok(normalized) => {
                if !key.trim().starts_with("0x") {
                    println!("  (added 0x prefix)");
                }
                return Ok(normalized);
            }
            Err(msg) => {
                println!("  {} Please try again.", msg);
                continue;
            }
        }
    }
}

/// Repeatedly prompt for all three CLOB credentials until valid.
fn read_clob_keys_loop() -> io::Result<(String, String, String)> {
    loop {
        let key = prompt_line("  API Key: ")?;
        if key.is_empty() {
            println!("  API Key cannot be empty. Please try again.");
            println!();
            continue;
        }

        let secret = rpassword::prompt_password("  API Secret (hidden): ")?;
        let secret = secret.trim().to_string();
        if secret.is_empty() {
            println!("  API Secret cannot be empty. Please try again.");
            println!();
            continue;
        }

        let passphrase = rpassword::prompt_password("  API Passphrase (hidden): ")?;
        let passphrase = passphrase.trim().to_string();
        if passphrase.is_empty() {
            println!("  API Passphrase cannot be empty. Please try again.");
            println!();
            continue;
        }

        return Ok((key, secret, passphrase));
    }
}

/// Validate an HTTP/HTTPS URL.
pub fn validate_url(input: &str) -> std::result::Result<String, String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err("URL cannot be empty.".into());
    }
    if !trimmed.starts_with("http://") && !trimmed.starts_with("https://") {
        return Err("URL must start with http:// or https://.".into());
    }
    Ok(trimmed.to_string())
}

/// Validate an Ethereum address (0x + 40 hex chars).
pub fn validate_eth_address(input: &str) -> std::result::Result<String, String> {
    let trimmed = input.trim();
    if !trimmed.starts_with("0x") || trimmed.len() != 42 {
        return Err("Address must be 0x followed by 40 hex characters.".into());
    }
    let hex_part = &trimmed[2..];
    if !hex_part.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err("Address contains non-hex characters.".into());
    }
    Ok(trimmed.to_string())
}

/// Prompt for an optional URL. Validates format if provided.
fn read_optional_url(msg: &str) -> io::Result<Option<String>> {
    loop {
        let input = prompt_line(msg)?;
        if input.is_empty() {
            return Ok(None);
        }
        match validate_url(&input) {
            Ok(u) => return Ok(Some(u)),
            Err(m) => {
                println!("  {} Please try again.", m);
                continue;
            }
        }
    }
}

/// Prompt for an optional Ethereum address. Validates format if provided.
fn read_optional_address(msg: &str) -> io::Result<Option<String>> {
    loop {
        let input = prompt_line(msg)?;
        if input.is_empty() {
            return Ok(None);
        }
        match validate_eth_address(&input) {
            Ok(a) => return Ok(Some(a)),
            Err(m) => {
                println!("  {} Please try again.", m);
                continue;
            }
        }
    }
}

// ── Display helpers ──────────────────────────────────────────────────────────

fn mask_secret(s: &str) -> String {
    if s.len() > 10 {
        format!("{}...{}", &s[..6], &s[s.len() - 4..])
    } else if s.len() > 4 {
        format!("{}...", &s[..4])
    } else {
        "****".to_string()
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() > max {
        format!("{}...", &s[..max])
    } else {
        s.to_string()
    }
}

// ── Existing config ──────────────────────────────────────────────────────────

/// In-memory snapshot of the auth-relevant fields of a poly config file.
/// Returned by [`load_existing`].
pub struct ExistingConfig {
    pub private_key: Option<String>,
    pub api_key: Option<String>,
    pub api_secret: Option<String>,
    pub api_passphrase: Option<String>,
    pub rpc_url: Option<String>,
    pub funder_address: Option<String>,
    pub tui_section: String,
}

/// Read an existing poly config file (TOML) and extract the auth section.
/// Missing files yield an all-`None` `ExistingConfig` (rather than an error).
pub fn load_existing(path: &PathBuf) -> ExistingConfig {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(_) => {
            return ExistingConfig {
                private_key: None,
                api_key: None,
                api_secret: None,
                api_passphrase: None,
                rpc_url: None,
                funder_address: None,
                tui_section: String::new(),
            };
        }
    };

    let tui_section = text
        .find("\n[tui]")
        .map(|i| text[i..].to_string())
        .or_else(|| {
            if text.starts_with("[tui]") {
                Some(text.clone())
            } else {
                None
            }
        })
        .unwrap_or_default();

    #[derive(serde::Deserialize, Default)]
    struct AuthSection {
        private_key: Option<String>,
        api_key: Option<String>,
        api_secret: Option<String>,
        api_passphrase: Option<String>,
        polygon_rpc_url: Option<String>,
        funder_address: Option<String>,
    }

    #[derive(serde::Deserialize, Default)]
    struct ConfigFile {
        auth: Option<AuthSection>,
        private_key: Option<String>,
        api_key: Option<String>,
        api_secret: Option<String>,
        api_passphrase: Option<String>,
        rpc_url: Option<String>,
        funder_address: Option<String>,
    }

    let cfg: ConfigFile = toml::from_str(&text).unwrap_or_default();
    let a = cfg.auth.as_ref();

    ExistingConfig {
        private_key: a.and_then(|a| a.private_key.clone()).or(cfg.private_key),
        api_key: a.and_then(|a| a.api_key.clone()).or(cfg.api_key),
        api_secret: a.and_then(|a| a.api_secret.clone()).or(cfg.api_secret),
        api_passphrase: a
            .and_then(|a| a.api_passphrase.clone())
            .or(cfg.api_passphrase),
        rpc_url: a.and_then(|a| a.polygon_rpc_url.clone()).or(cfg.rpc_url),
        funder_address: a
            .and_then(|a| a.funder_address.clone())
            .or(cfg.funder_address),
        tui_section,
    }
}

// ── Config write path ────────────────────────────────────────────────────────

fn config_write_path() -> PathBuf {
    if let Ok(p) = std::env::var("POLY_CONFIG") {
        return PathBuf::from(p);
    }
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    let xdg = std::env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| home.join(".config"));
    let xdg_path = xdg.join("poly").join("config.toml");
    if xdg_path.exists() {
        return xdg_path;
    }
    let legacy = home.join(".poly").join("config.toml");
    if legacy.exists() {
        return legacy;
    }
    legacy
}

// ── Config writing ───────────────────────────────────────────────────────────

/// Write a poly config file, creating parent directories as needed.
/// On Unix the file is chmod'd to 0o600 and the parent directory to 0o700.
#[allow(clippy::too_many_arguments)]
pub fn write_config(
    path: &PathBuf,
    private_key: &str,
    api_key: &str,
    api_secret: &str,
    api_passphrase: &str,
    rpc_url: Option<&str>,
    funder_address: Option<&str>,
    tui_section: &str,
) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700));
        }
    }

    let mut content = String::new();
    content.push_str("[auth]\n");
    content.push_str(&format!("private_key    = \"{}\"\n", private_key));
    content.push_str(&format!("api_key        = \"{}\"\n", api_key));
    content.push_str(&format!("api_secret     = \"{}\"\n", api_secret));
    content.push_str(&format!("api_passphrase = \"{}\"\n", api_passphrase));
    if let Some(rpc) = rpc_url {
        content.push_str(&format!("polygon_rpc_url = \"{}\"\n", rpc));
    }
    if let Some(funder) = funder_address {
        content.push_str(&format!("funder_address = \"{}\"\n", funder));
    }

    if !tui_section.is_empty() {
        content.push_str(tui_section);
        if !tui_section.ends_with('\n') {
            content.push('\n');
        }
    }

    std::fs::write(path, &content)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }

    Ok(())
}

/// Exposed for TUI setup screen to reuse path resolution logic.
pub fn config_write_path_for_tui() -> PathBuf {
    config_write_path()
}

/// Check whether credentials are available from any source (env vars, .env, config file).
pub fn has_config() -> bool {
    if std::env::var("POLY_PRIVATE_KEY").is_ok() || std::env::var("POLY_MARKET_KEY").is_ok() {
        return true;
    }
    let home = match dirs::home_dir() {
        Some(h) => h,
        None => return false,
    };
    if let Ok(p) = std::env::var("POLY_CONFIG") {
        return std::path::Path::new(&p).exists();
    }
    let xdg = std::env::var("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| home.join(".config"));
    if xdg.join("poly").join("config.toml").exists() {
        return true;
    }
    home.join(".poly").join("config.toml").exists()
}
