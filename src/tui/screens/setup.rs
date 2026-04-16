use ratatui::{
    layout::{Constraint, Layout, Margin, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Clear, Paragraph},
    Frame,
};

use crate::tui::theme;

#[derive(Debug, Clone, PartialEq)]
pub enum SetupStep {
    PrivateKey,
    ApiKey,
    ApiSecret,
    ApiPassphrase,
    RpcUrl,
    FunderAddress,
    Confirm,
}

impl SetupStep {
    fn index(&self) -> usize {
        match self {
            SetupStep::PrivateKey => 0,
            SetupStep::ApiKey => 1,
            SetupStep::ApiSecret => 2,
            SetupStep::ApiPassphrase => 3,
            SetupStep::RpcUrl => 4,
            SetupStep::FunderAddress => 5,
            SetupStep::Confirm => 6,
        }
    }

    fn next(&self) -> SetupStep {
        match self {
            SetupStep::PrivateKey => SetupStep::ApiKey,
            SetupStep::ApiKey => SetupStep::ApiSecret,
            SetupStep::ApiSecret => SetupStep::ApiPassphrase,
            SetupStep::ApiPassphrase => SetupStep::RpcUrl,
            SetupStep::RpcUrl => SetupStep::FunderAddress,
            SetupStep::FunderAddress => SetupStep::Confirm,
            SetupStep::Confirm => SetupStep::Confirm,
        }
    }

    fn prev(&self) -> SetupStep {
        match self {
            SetupStep::PrivateKey => SetupStep::PrivateKey,
            SetupStep::ApiKey => SetupStep::PrivateKey,
            SetupStep::ApiSecret => SetupStep::ApiKey,
            SetupStep::ApiPassphrase => SetupStep::ApiSecret,
            SetupStep::RpcUrl => SetupStep::ApiPassphrase,
            SetupStep::FunderAddress => SetupStep::RpcUrl,
            SetupStep::Confirm => SetupStep::FunderAddress,
        }
    }
}

const TOTAL_STEPS: usize = 7;

#[derive(Debug, Clone)]
pub struct SetupForm {
    pub step: SetupStep,
    pub private_key: String,
    pub api_key: String,
    pub api_secret: String,
    pub api_passphrase: String,
    pub rpc_url: String,
    pub funder_address: String,
    pub error: Option<String>,
    pub is_first_launch: bool,
}

impl Default for SetupForm {
    fn default() -> Self {
        Self {
            step: SetupStep::PrivateKey,
            private_key: String::new(),
            api_key: String::new(),
            api_secret: String::new(),
            api_passphrase: String::new(),
            rpc_url: String::new(),
            funder_address: String::new(),
            error: None,
            is_first_launch: false,
        }
    }
}

impl SetupForm {
    pub fn current_input(&self) -> &str {
        match self.step {
            SetupStep::PrivateKey => &self.private_key,
            SetupStep::ApiKey => &self.api_key,
            SetupStep::ApiSecret => &self.api_secret,
            SetupStep::ApiPassphrase => &self.api_passphrase,
            SetupStep::RpcUrl => &self.rpc_url,
            SetupStep::FunderAddress => &self.funder_address,
            SetupStep::Confirm => "",
        }
    }

    pub fn push_char(&mut self, c: char) {
        self.error = None;
        match self.step {
            SetupStep::PrivateKey => self.private_key.push(c),
            SetupStep::ApiKey => self.api_key.push(c),
            SetupStep::ApiSecret => self.api_secret.push(c),
            SetupStep::ApiPassphrase => self.api_passphrase.push(c),
            SetupStep::RpcUrl => self.rpc_url.push(c),
            SetupStep::FunderAddress => self.funder_address.push(c),
            SetupStep::Confirm => {}
        }
    }

    pub fn backspace(&mut self) {
        self.error = None;
        match self.step {
            SetupStep::PrivateKey => {
                self.private_key.pop();
            }
            SetupStep::ApiKey => {
                self.api_key.pop();
            }
            SetupStep::ApiSecret => {
                self.api_secret.pop();
            }
            SetupStep::ApiPassphrase => {
                self.api_passphrase.pop();
            }
            SetupStep::RpcUrl => {
                self.rpc_url.pop();
            }
            SetupStep::FunderAddress => {
                self.funder_address.pop();
            }
            SetupStep::Confirm => {}
        }
    }

    /// Validate the current step and advance to the next. Returns true if setup is complete.
    pub fn advance(&mut self) -> bool {
        self.error = None;

        match self.step {
            SetupStep::PrivateKey => {
                let key = self.private_key.trim().to_string();
                if key.is_empty() {
                    self.error = Some("Private key is required".into());
                    return false;
                }
                // Auto-fix missing 0x prefix
                if key.len() == 64 && key.chars().all(|c| c.is_ascii_hexdigit()) {
                    self.private_key = format!("0x{}", key);
                } else if !key.starts_with("0x") {
                    self.error = Some("Must start with 0x".into());
                    return false;
                } else {
                    let hex = &key[2..];
                    if hex.len() != 64 {
                        self.error =
                            Some(format!("Expected 64 hex chars after 0x, got {}", hex.len()));
                        return false;
                    }
                    if !hex.chars().all(|c| c.is_ascii_hexdigit()) {
                        self.error = Some("Contains non-hex characters".into());
                        return false;
                    }
                    self.private_key = key;
                }
            }
            SetupStep::ApiKey => {
                if self.api_key.trim().is_empty() {
                    self.error =
                        Some("API key is required. Run `poly derive-keys` to generate one.".into());
                    return false;
                }
            }
            SetupStep::ApiSecret => {
                if self.api_secret.trim().is_empty() {
                    self.error = Some("API secret is required".into());
                    return false;
                }
            }
            SetupStep::ApiPassphrase => {
                if self.api_passphrase.trim().is_empty() {
                    self.error = Some("API passphrase is required".into());
                    return false;
                }
            }
            SetupStep::RpcUrl => {
                let url = self.rpc_url.trim().to_string();
                if !url.is_empty() && !url.starts_with("http://") && !url.starts_with("https://") {
                    self.error = Some("URL must start with http:// or https://".into());
                    return false;
                }
                self.rpc_url = url;
            }
            SetupStep::FunderAddress => {
                let addr = self.funder_address.trim().to_string();
                if !addr.is_empty() {
                    if !addr.starts_with("0x") || addr.len() != 42 {
                        self.error = Some("Must be 0x followed by 40 hex characters".into());
                        return false;
                    }
                    if !addr[2..].chars().all(|c| c.is_ascii_hexdigit()) {
                        self.error = Some("Contains non-hex characters".into());
                        return false;
                    }
                }
                self.funder_address = addr;
            }
            SetupStep::Confirm => {
                return true;
            }
        }

        self.step = self.step.next();
        false
    }

    pub fn go_back(&mut self) {
        self.error = None;
        self.step = self.step.prev();
    }

    /// Write the config file. Returns Ok(path) on success.
    pub fn save(&self) -> std::io::Result<std::path::PathBuf> {
        let path = crate::setup::config_write_path_for_tui();

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700));
            }
        }

        // Preserve existing [tui] section if present
        let tui_section = std::fs::read_to_string(&path)
            .ok()
            .and_then(|text| text.find("\n[tui]").map(|i| text[i..].to_string()))
            .unwrap_or_default();

        let mut content = String::new();
        content.push_str("[auth]\n");
        content.push_str(&format!("private_key    = \"{}\"\n", self.private_key));
        content.push_str(&format!("api_key        = \"{}\"\n", self.api_key));
        content.push_str(&format!("api_secret     = \"{}\"\n", self.api_secret));
        content.push_str(&format!("api_passphrase = \"{}\"\n", self.api_passphrase));
        if !self.rpc_url.is_empty() {
            content.push_str(&format!("polygon_rpc_url = \"{}\"\n", self.rpc_url));
        }
        if !self.funder_address.is_empty() {
            content.push_str(&format!("funder_address = \"{}\"\n", self.funder_address));
        }

        if !tui_section.is_empty() {
            content.push_str(&tui_section);
            if !tui_section.ends_with('\n') {
                content.push('\n');
            }
        }

        std::fs::write(&path, &content)?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
        }

        Ok(path)
    }
}

// ── Rendering ────────────────────────────────────────────────────────────────

pub fn render(f: &mut Frame, area: Rect, form: &SetupForm) {
    f.render_widget(Clear, area);

    let bg = Block::default().style(Style::default().bg(theme::BG));
    f.render_widget(bg, area);

    let center = centered_box(60, 24, area);

    let block = Block::bordered()
        .title(Span::styled(
            " Setup ",
            Style::default()
                .fg(theme::CYAN)
                .add_modifier(Modifier::BOLD),
        ))
        .border_style(Style::default().fg(theme::BORDER))
        .style(Style::default().bg(theme::PANEL_BG));

    let inner = block.inner(center);
    f.render_widget(block, center);

    let content = inner.inner(Margin {
        horizontal: 2,
        vertical: 0,
    });

    let rows = Layout::vertical([
        Constraint::Length(2), // header
        Constraint::Length(2), // description
        Constraint::Length(1), // spacer
        Constraint::Length(1), // progress
        Constraint::Length(1), // spacer
        Constraint::Min(6),    // fields
        Constraint::Length(1), // error
        Constraint::Length(1), // spacer
        Constraint::Length(1), // footer
    ])
    .split(content);

    // Header
    let title = if form.is_first_launch {
        "Welcome to poly! Let's set up your credentials."
    } else {
        "Configure your trading credentials."
    };
    f.render_widget(
        Paragraph::new(title).style(Style::default().fg(theme::TEXT)),
        rows[0],
    );

    // Step description
    let (desc, hint) = step_description(&form.step);
    let mut desc_lines = vec![Line::from(Span::styled(
        desc,
        Style::default().fg(theme::DIM),
    ))];
    if !hint.is_empty() {
        desc_lines.push(Line::from(Span::styled(
            hint,
            Style::default().fg(theme::VERY_DIM),
        )));
    }
    f.render_widget(Paragraph::new(desc_lines), rows[1]);

    // Progress bar
    let step_idx = form.step.index();
    let progress = format!("Step {}/{}", step_idx + 1, TOTAL_STEPS,);
    let bar = render_progress_bar(step_idx, TOTAL_STEPS, rows[3].width as usize);
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(progress, Style::default().fg(theme::CYAN)),
            Span::raw("  "),
            bar,
        ])),
        rows[3],
    );

    // Fields
    render_fields(f, rows[5], form);

    // Error message
    if let Some(ref err) = form.error {
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                err.as_str(),
                Style::default().fg(theme::RED),
            ))),
            rows[6],
        );
    }

    // Footer
    let footer = if form.step == SetupStep::Confirm {
        Line::from(vec![
            Span::styled("Enter", Style::default().fg(theme::CYAN)),
            Span::styled(" save & restart  ", Style::default().fg(theme::HINT)),
            Span::styled("Backspace", Style::default().fg(theme::CYAN)),
            Span::styled(" back  ", Style::default().fg(theme::HINT)),
            Span::styled("Esc", Style::default().fg(theme::CYAN)),
            Span::styled(" cancel", Style::default().fg(theme::HINT)),
        ])
    } else {
        Line::from(vec![
            Span::styled("Enter", Style::default().fg(theme::CYAN)),
            Span::styled(" next  ", Style::default().fg(theme::HINT)),
            Span::styled("Backspace/←", Style::default().fg(theme::CYAN)),
            Span::styled(" edit  ", Style::default().fg(theme::HINT)),
            Span::styled("Shift+Tab", Style::default().fg(theme::CYAN)),
            Span::styled(" back  ", Style::default().fg(theme::HINT)),
            Span::styled("Esc", Style::default().fg(theme::CYAN)),
            Span::styled(" cancel", Style::default().fg(theme::HINT)),
        ])
    };
    f.render_widget(Paragraph::new(footer), rows[8]);
}

fn render_fields(f: &mut Frame, area: Rect, form: &SetupForm) {
    let fields: Vec<(&str, &str, bool, bool)> = vec![
        (
            "Private Key",
            &form.private_key,
            true,
            form.step == SetupStep::PrivateKey,
        ),
        (
            "API Key",
            &form.api_key,
            false,
            form.step == SetupStep::ApiKey,
        ),
        (
            "API Secret",
            &form.api_secret,
            true,
            form.step == SetupStep::ApiSecret,
        ),
        (
            "API Passphrase",
            &form.api_passphrase,
            true,
            form.step == SetupStep::ApiPassphrase,
        ),
        (
            "RPC URL (optional)",
            &form.rpc_url,
            false,
            form.step == SetupStep::RpcUrl,
        ),
        (
            "Funder Address (optional)",
            &form.funder_address,
            false,
            form.step == SetupStep::FunderAddress,
        ),
    ];

    let constraints: Vec<Constraint> = fields.iter().map(|_| Constraint::Length(1)).collect();
    let rows = Layout::vertical(constraints).split(area);

    for (i, (label, value, secret, active)) in fields.iter().enumerate() {
        let step = match i {
            0 => SetupStep::PrivateKey,
            1 => SetupStep::ApiKey,
            2 => SetupStep::ApiSecret,
            3 => SetupStep::ApiPassphrase,
            4 => SetupStep::RpcUrl,
            5 => SetupStep::FunderAddress,
            _ => unreachable!(),
        };

        let done = step.index() < form.step.index();
        let masked_buf;
        let display_val: &str = if value.is_empty() {
            if *active {
                "█"
            } else {
                ""
            }
        } else if *secret && !*active {
            masked_buf = mask_str(value);
            &masked_buf
        } else {
            value
        };

        let (label_style, val_style) = if *active {
            (
                Style::default()
                    .fg(theme::CYAN)
                    .add_modifier(Modifier::BOLD),
                Style::default().fg(theme::TEXT),
            )
        } else if done {
            (
                Style::default().fg(theme::DIM),
                Style::default().fg(theme::DIM),
            )
        } else {
            (
                Style::default().fg(theme::VERY_DIM),
                Style::default().fg(theme::VERY_DIM),
            )
        };

        let check = if done {
            "✓ "
        } else if *active {
            "> "
        } else {
            "  "
        };
        let check_style = if done {
            Style::default().fg(Color::Rgb(62, 224, 126))
        } else if *active {
            Style::default().fg(theme::CYAN)
        } else {
            Style::default().fg(theme::VERY_DIM)
        };

        // For secret active fields, mask all but cursor position
        let shown_val = if *active && *secret && !value.is_empty() {
            let masked: String = "*".repeat(value.len());
            format!("{}█", masked)
        } else if *active && !value.is_empty() {
            format!("{}█", display_val)
        } else {
            display_val.to_string()
        };

        let line = Line::from(vec![
            Span::styled(check, check_style),
            Span::styled(format!("{:<26} ", label), label_style),
            Span::styled(shown_val, val_style),
        ]);

        f.render_widget(Paragraph::new(line), rows[i]);
    }
}

fn step_description(step: &SetupStep) -> (&'static str, &'static str) {
    match step {
        SetupStep::PrivateKey => (
            "Your Ethereum private key signs orders on Polymarket.",
            "Export from MetaMask: Account menu > Account details > Show private key",
        ),
        SetupStep::ApiKey => (
            "CLOB API Key for authenticated requests.",
            "Generate with `poly derive-keys` or from your Polymarket API settings",
        ),
        SetupStep::ApiSecret => ("CLOB API Secret (used for HMAC signing).", ""),
        SetupStep::ApiPassphrase => ("CLOB API Passphrase.", ""),
        SetupStep::RpcUrl => (
            "Polygon RPC URL — needed only for balance checks.",
            "Get a free key at alchemy.com or infura.io (press Enter to skip)",
        ),
        SetupStep::FunderAddress => (
            "Proxy/Gnosis Safe address — most users should skip this.",
            "Press Enter to skip",
        ),
        SetupStep::Confirm => (
            "Review your settings and press Enter to save.",
            "poly will restart to load the new configuration.",
        ),
    }
}

fn mask_str(s: &str) -> String {
    if s.len() > 8 {
        format!("{}...{}", &s[..4], &s[s.len() - 4..])
    } else {
        "*".repeat(s.len())
    }
}

fn render_progress_bar(current: usize, total: usize, width: usize) -> Span<'static> {
    let bar_width = width.saturating_sub(12).min(30);
    let filled = if total > 0 {
        (current * bar_width) / total
    } else {
        0
    };
    let empty = bar_width.saturating_sub(filled);
    let bar = format!("[{}{}]", "=".repeat(filled), " ".repeat(empty));
    Span::styled(bar, Style::default().fg(theme::DIM))
}

fn centered_box(w: u16, h: u16, r: Rect) -> Rect {
    let clamped_w = w.min(r.width);
    let clamped_h = h.min(r.height);
    let x = r.x + (r.width.saturating_sub(clamped_w)) / 2;
    let y = r.y + (r.height.saturating_sub(clamped_h)) / 2;
    Rect::new(x, y, clamped_w, clamped_h)
}
