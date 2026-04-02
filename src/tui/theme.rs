#![allow(dead_code)]
use ratatui::style::Color;

// ── Background ────────────────────────────────────────────────────────────────
pub const BG: Color = Color::Rgb(20, 20, 32);
pub const PANEL_BG: Color = Color::Rgb(28, 28, 42);

// ── Borders ───────────────────────────────────────────────────────────────────
pub const BORDER: Color = Color::Rgb(70, 70, 110);
pub const BORDER_ACTIVE: Color = Color::Rgb(100, 100, 200);
pub const BORDER_WARNING: Color = Color::Rgb(200, 130, 30);

// ── Text ──────────────────────────────────────────────────────────────────────
pub const TEXT: Color = Color::Rgb(220, 220, 235);
pub const DIM: Color = Color::Rgb(140, 140, 170);
pub const VERY_DIM: Color = Color::Rgb(90, 90, 110);

// ── Semantic ──────────────────────────────────────────────────────────────────
pub const GREEN: Color = Color::Rgb(100, 210, 140);
pub const RED: Color = Color::Rgb(230, 120, 120);
pub const CYAN: Color = Color::Rgb(0, 200, 210);
pub const YELLOW: Color = Color::Rgb(230, 160, 60);
pub const BLUE: Color = Color::Rgb(100, 180, 255);
pub const PURPLE: Color = Color::Rgb(180, 140, 255);
pub const ERROR: Color = Color::Rgb(240, 90, 90);
