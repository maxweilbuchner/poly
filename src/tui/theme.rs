#![allow(dead_code)]
use ratatui::style::Color;

// ── Background ────────────────────────────────────────────────────────────────
pub const BG: Color = Color::Rgb(10, 10, 18);
pub const PANEL_BG: Color = Color::Rgb(17, 17, 30);

// ── Borders ───────────────────────────────────────────────────────────────────
pub const BORDER: Color = Color::Rgb(48, 50, 88);
pub const BORDER_ACTIVE: Color = Color::Rgb(82, 115, 230);
pub const BORDER_WARNING: Color = Color::Rgb(210, 140, 28);

// ── Text ──────────────────────────────────────────────────────────────────────
pub const TEXT: Color = Color::Rgb(215, 220, 240);
pub const DIM: Color = Color::Rgb(125, 128, 158);
pub const VERY_DIM: Color = Color::Rgb(64, 66, 90);
pub const HINT: Color = Color::Rgb(88, 90, 118);

// ── Semantic ──────────────────────────────────────────────────────────────────
pub const GREEN: Color = Color::Rgb(62, 224, 126);
pub const RED: Color = Color::Rgb(240, 80, 80);
pub const CYAN: Color = Color::Rgb(0, 205, 218);
pub const YELLOW: Color = Color::Rgb(238, 172, 50);
pub const BLUE: Color = Color::Rgb(82, 168, 252);
pub const PURPLE: Color = Color::Rgb(170, 126, 252);
pub const ERROR: Color = Color::Rgb(246, 62, 62);
