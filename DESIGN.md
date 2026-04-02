# poly — UI/UX Design Principles

This file is the single source of truth for visual and interaction design in `poly`.
Claude must read and apply these rules whenever working on TUI, display, or output code.

---

## 1. Color Palette

All colors use RGB (ratatui `Color::Rgb`), never raw ANSI colors. This ensures
consistent rendering across dark terminals.

### Base
| Role              | RGB                | Usage                                      |
|-------------------|--------------------|--------------------------------------------|
| Background        | `(20, 20, 32)`     | Terminal background / full-screen fill     |
| Panel background  | `(28, 28, 42)`     | Block/widget inner background              |
| Border default    | `(70, 70, 110)`    | Block borders in normal state              |
| Border active     | `(100, 100, 200)`  | Focused / selected block border            |
| Border warning    | `(200, 130, 30)`   | Scrolled-away or stale state               |
| Border paused     | Magenta            | Any component in a paused/disabled state   |

### Text
| Role           | RGB                | Usage                                       |
|----------------|--------------------|---------------------------------------------|
| Primary        | `(220, 220, 235)`  | Main readable values                        |
| Label / dim    | `(140, 140, 170)`  | Field labels, column headers                |
| Very dim       | `(90, 90, 110)`    | Timestamps, secondary metadata              |
| Timestamp      | `(80, 80, 105)`    | Log line timestamps only                    |

### Semantic
| Role           | RGB / Named         | Usage                                      |
|----------------|---------------------|--------------------------------------------|
| Profit / UP    | `(100, 210, 140)`   | Positive PnL, buy side, UP outcomes        |
| Loss / DOWN    | `(230, 120, 120)`   | Negative PnL, sell side, DOWN outcomes     |
| Order confirm  | `(100, 180, 255)`   | Order placed / live order status           |
| Signal         | `(180, 140, 255)`   | Trading signals, maker order notes         |
| Warning        | `(230, 160, 60)`    | Non-critical warnings, paused state text   |
| Error          | `(240, 90, 90)`     | Errors, failures                           |
| System / info  | `(90, 110, 140)`    | System messages, neutral logs              |
| Cyan accent    | Cyan                | Interactive elements, titles, tab labels   |
| Yellow accent  | Yellow              | Volume figures, spread, highlight          |

---

## 2. Layout Rules

### Panel structure (TUI full-screen)
```
┌─ Tab Bar ──────────────────────────────────────────────────────┐  height: 1
├─ Main Content Area ───────────────────────────────────────────┤  Min(0)
│  (screen-specific panels stacked vertically)                  │
├─ Status / Footer ─────────────────────────────────────────────┤  height: 1
└────────────────────────────────────────────────────────────────┘
```

- **Tab bar** (top, height 1): active tab bold + cyan, others dim. No borders.
- **Status bar** (bottom, height 1): key hints left-aligned, flash messages center-aligned. No borders.
- **Content panels**: use `Constraint::Length(n)` for fixed-height panels, `Constraint::Min(0)` for the primary scrollable area.
- Never nest more than two levels of layout constraints.

### Modal overlays
- Centered via percentage: 40–65% width, 12–20% height depending on content.
- Always render a `Clear` widget first to avoid bleed-through.
- Border color: cyan. Title color: cyan bold.
- Footer row inside modal: dim gray key hint text.
- Selected item cursor: `▸` in the color of that option.

### Block / border convention
- All panels wrapped in `Block::bordered()` with a short title.
- Title format: `" Title "` (single space padding each side).
- When a panel is scrolled away from the bottom, append ` ↑N` to the title and change border to warning color.

---

## 3. Typography & Symbols

- All output is monospace — no assumptions about proportional fonts.
- **Bold** for: prices, PnL values, active menu items, selected rows, column headers.
- **Dim** for: metadata, timestamps, secondary info.
- Prefer Unicode over ASCII for UI chrome:
  - Separators: `─`, `│`, `═`
  - Cursor: `▸`
  - Direction: `↑` `↓`
  - Timer: `⏱`
  - Input caret: `▏`
- Keep column widths fixed — use `truncate()` with `…` for overflow, never wrap mid-word.
- Volume / liquidity display rules:
  - ≥ 1 000 000 → `$X.XM`
  - ≥ 1 000     → `$X.XK`
  - otherwise   → `$X.XX`

---

## 4. Navigation Rules

### Modal state machine

```
Root View
  └─ q / Ctrl+C → Root Menu modal
       ├─ Quit
       ├─ Back / Cancel  (Esc also works)
       └─ Help  → Help overlay (Esc to close)

Within a list screen
  ├─ ↑ ↓ / j k      → move selection
  ├─ Enter           → drill in (push screen)
  ├─ Esc / h         → go back (pop screen)
  ├─ /               → activate search / filter mode
  └─ r               → refresh data

Within a modal form (order entry, settings)
  ├─ Tab / Shift+Tab → cycle fields
  ├─ Enter           → confirm / submit
  └─ Esc             → cancel, close modal
```

### Principles
- **Never dead-end the user.** Every screen must have a visible way back (Esc, `h`, or Back item).
- **Esc is always safe.** It must never submit, delete, or confirm — only cancel/go back.
- **`q` opens a menu, it does not immediately quit.** Immediate destructive quit is only from within the menu.
- **Arrow keys and vim keys (`jk`) are always equivalent** for list navigation.
- **Tab cycles forward**, Shift+Tab cycles backward, within forms and between panels.
- Global keys (`1`, `2`, `3` for tabs; `r` for refresh; `?` for help) work from any non-editing state.

---

## 5. Log Colorization

Log lines follow the format: `[HH:MM:SS] MESSAGE`

Timestamp portion always rendered in timestamp color `(80, 80, 105)`.

Message colorization (first match wins):

| Prefix / keyword      | Color                   |
|-----------------------|-------------------------|
| `PROFIT:`             | `(80, 200, 120)` bold   |
| `LOSS:`               | `(240, 90, 90)` bold    |
| `LIVE ERROR` / `Failed` | `(240, 90, 90)`       |
| `LIVE:` (order placed)| `(100, 180, 255)`       |
| `SIGNAL:` / `MAKER:`  | `(180, 140, 255)`       |
| `Trading PAUSED`      | `(230, 160, 60)`        |
| `Trading RESUMED`     | `(80, 200, 120)`        |
| `Optimizing` / `updated` | Cyan                 |
| `ERROR` / `Failed` (substring) | `(240, 90, 90)` |
| `System` (prefix)     | `(90, 110, 140)`        |
| (default)             | `(140, 140, 170)`       |

Log buffer: max **200 lines** (VecDeque). Older entries dropped from the front.

---

## 6. Flash Messages

- Displayed in the status bar, center-aligned.
- Auto-expire after **3 seconds** (`Instant`-based, checked on each render).
- Examples: `"Order placed: {id}"`, `"Copied to clipboard"`, `"Settings updated"`.
- Never block the UI — rendered inline in the footer row.

---

## 7. CLI Output (non-TUI commands)

For one-shot CLI commands (`poly search`, `poly orders`, etc.):

- Use `colored` crate with the semantic palette above where possible (ANSI fallback is acceptable here).
- Info prefix: `→` dimmed, followed by message.
- Error prefix: `error:` bold red, to stderr.
- Tables: fixed-width columns, `─` separator line under header, `…` truncation.
- Avoid trailing blank lines — one blank line after a block/table is the maximum.

---

## 8. General Principles

1. **Information density over decoration.** Every pixel/character must carry signal. No purely decorative borders or padding beyond one space.
2. **Consistent affordances.** If `Enter` confirms in one modal, it confirms in all modals. No exceptions.
3. **Visible state.** Paused, loading, error, and scrolled states must all be visually distinct — never silent.
4. **No surprise side effects.** Destructive actions (cancel order, cancel-all) require a confirmation step.
5. **Graceful degradation.** If an API call fails, show the last known data with a warning indicator — do not crash or blank the screen.
6. **Auth-aware rendering.** Screens that require credentials should show a clear, helpful message (not a raw error) when credentials are missing.
