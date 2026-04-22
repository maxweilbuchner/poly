#!/usr/bin/env python3
"""
Generate an ANSI-colored mock of the poly TUI Markets tab.
Piped through `freeze` to produce docs/screenshot.png.

Colors match src/tui/theme.rs. This is a static mock, not a live capture —
the shape, columns, and colors are hand-assembled to represent a realistic
Markets tab frame. Re-generate with:

    python3 docs/gen_screenshot.py | freeze \
        --language ansi \
        --font.family "JetBrains Mono" \
        --font.size 13 \
        --line-height 1.3 \
        --padding 20 \
        --border.radius 8 \
        --background "#0a0a12" \
        --window \
        -o docs/screenshot.png
"""

ESC = "\x1b"
RESET = f"{ESC}[0m"


def fg(r, g, b): return f"{ESC}[38;2;{r};{g};{b}m"
def bg(r, g, b): return f"{ESC}[48;2;{r};{g};{b}m"
def bold(s): return f"{ESC}[1m{s}{ESC}[22m"


# theme.rs colors
BG = bg(10, 10, 18)
PANEL = bg(17, 17, 30)
TEXT = fg(215, 220, 240)
DIM = fg(125, 128, 158)
VERY_DIM = fg(64, 66, 90)
BORDER = fg(48, 50, 88)
BORDER_ACTIVE = fg(82, 115, 230)
CYAN = fg(0, 205, 218)
GREEN = fg(62, 224, 126)
RED = fg(240, 80, 80)
YELLOW = fg(238, 172, 50)
BLUE = fg(82, 168, 252)


def tab_bar():
    # Active tab: cyan bold; inactive: dim.
    parts = []
    tabs = [("1", "Markets"), ("2", "Positions"), ("3", "Balance"),
            ("4", "Analytics"), ("5", "Viewer")]
    for i, (key, name) in enumerate(tabs):
        if i == 0:
            parts.append(f" {CYAN}{bold(key)} {bold(name)}{RESET}{PANEL} ")
        else:
            parts.append(f" {DIM}{key} {name}{RESET}{PANEL} ")
    line = "".join(parts)
    return line


def row(rank, q, vol, end, pct, trend, highlight=False, has_pos=False):
    # Probability bar: filled squares (▓) + light (░).
    filled = int(round(pct / 10))
    bar_cells = "▓" * filled + "░" * (10 - filled)
    if pct >= 60:
        bar_color = GREEN
    elif pct >= 30:
        bar_color = YELLOW
    else:
        bar_color = RED

    # Trend arrow.
    if trend > 0:
        arrow = f"{GREEN}▲ +{trend:.0f}%{RESET}{PANEL}"
    elif trend < 0:
        arrow = f"{RED}▼ {trend:.0f}%{RESET}{PANEL}"
    else:
        arrow = f"{DIM}·  0%{RESET}{PANEL}"

    cursor = f"{CYAN}▸{RESET}{PANEL}" if highlight else " "
    rank_str = f"{DIM}{rank:>2}{RESET}{PANEL}"
    q_styled = f"{TEXT}{bold(q) if highlight else q}{RESET}{PANEL}"
    q_pad = q[:46].ljust(46)
    q_styled = f"{TEXT}{bold(q_pad) if highlight else q_pad}{RESET}{PANEL}"
    vol_styled = f"{DIM}{vol:>7}{RESET}{PANEL}"
    end_styled = f"{DIM}{end:>8}{RESET}{PANEL}"
    bar_styled = f"{bar_color}{bar_cells}{RESET}{PANEL}"
    pct_styled = f"{TEXT}{pct:>3}%{RESET}{PANEL}"
    pos_tag = f"{CYAN}★{RESET}{PANEL}" if has_pos else " "

    return (f" {cursor} {rank_str}  {q_styled}  {vol_styled}  {end_styled}  "
            f"{bar_styled} {pct_styled}  {arrow}  {pos_tag}")


def border_line(left, right, char="─", width=110):
    return f"{BORDER}{left}{char * (width - 2)}{right}{RESET}"


def side(inner, width=110):
    # Pad inner (which may include ANSI) to visible width and wrap with panel bg + borders.
    # We count visible length by stripping ANSI — simple approx assuming no nested escapes.
    import re
    visible = re.sub(r"\x1b\[[0-9;]*m", "", inner)
    pad = width - 2 - len(visible)
    if pad < 0:
        pad = 0
    return f"{BORDER}│{RESET}{PANEL}{inner}{' ' * pad}{RESET}{BORDER}│{RESET}"


def main():
    W = 110
    out = []

    # Top border with title
    title = f" {CYAN}{bold('poly')} {DIM}— Polymarket CLI/TUI{RESET} "
    # Title sits inside the top border
    top = f"{BORDER}╭──{title}{BORDER}" + "─" * (W - 4 - len("  — Polymarket CLI/TUI  ") - len(" poly ")) + f"╮{RESET}"
    out.append(top)

    # Tab bar
    out.append(side(tab_bar()))

    # Separator
    out.append(f"{BORDER}├{'─' * (W - 2)}┤{RESET}")

    # Column headers
    header = (f" {DIM}   #  Question                                       "
              f"Volume      Ends  Probability          Chg   {RESET}{PANEL}")
    out.append(side(header))
    out.append(f"{BORDER}│{RESET}{PANEL}{' ' * (W - 2)}{RESET}{BORDER}│{RESET}")

    # Rows
    rows = [
        (1, "Will Bitcoin close above $200k in 2026?", "$12.4M", "Dec 31", 58, 4, True,  True),
        (2, "US recession declared by Q4 2026?",        "$8.1M",  "Dec 31", 24, -3, False, False),
        (3, "Fed cuts rates 25bps at May meeting?",     "$6.9M",  "May 07", 71, 12, False, True),
        (4, "Will SpaceX reach Mars orbit by 2027?",    "$4.7M",  "Dec 31",  9, 0,  False, False),
        (5, "NVIDIA passes Apple market cap in 2026?",  "$3.8M",  "Dec 31", 42, -2, False, False),
        (6, "Oscar Best Picture 2027: Dune Part Three", "$2.2M",  "Mar 15", 18, 1,  False, False),
        (7, "EU passes comprehensive AI Act by 2026?",  "$1.9M",  "Dec 31", 82, 3,  False, False),
        (8, "Will ETH/BTC ratio exceed 0.05 in 2026?",  "$1.4M",  "Dec 31", 34, -5, False, False),
    ]
    for r in rows:
        out.append(side(row(*r)))

    # Filler
    for _ in range(2):
        out.append(f"{BORDER}│{RESET}{PANEL}{' ' * (W - 2)}{RESET}{BORDER}│{RESET}")

    # Status bar separator
    out.append(f"{BORDER}├{'─' * (W - 2)}┤{RESET}")

    # Status / keybinds line
    hint = (f" {DIM}/{RESET}{PANEL}{TEXT} search  {DIM}s{RESET}{PANEL}{TEXT} sort  "
            f"{DIM}d/p/v{RESET}{PANEL}{TEXT} filter  {DIM}*{RESET}{PANEL}{TEXT} star  "
            f"{DIM}w{RESET}{PANEL}{TEXT} watchlist  {DIM}Enter{RESET}{PANEL}{TEXT} open  "
            f"{DIM}?{RESET}{PANEL}{TEXT} help {RESET}{PANEL}")
    out.append(side(hint))

    # Bottom status
    status = (f" {GREEN}●{RESET}{PANEL} {DIM}WS connected{RESET}{PANEL}   "
              f"{DIM}balance{RESET}{PANEL} {TEXT}$124.53{RESET}{PANEL}   "
              f"{DIM}allowance{RESET}{PANEL} {GREEN}unlimited{RESET}{PANEL}   "
              f"{DIM}positions{RESET}{PANEL} {TEXT}7{RESET}{PANEL}   "
              f"{DIM}net worth{RESET}{PANEL} {TEXT}$1,842.19{RESET}{PANEL} {GREEN}(+4.2%){RESET}{PANEL}")
    out.append(side(status))

    # Bottom border
    out.append(f"{BORDER}╰{'─' * (W - 2)}╯{RESET}")

    print("\n".join(out))


if __name__ == "__main__":
    main()
