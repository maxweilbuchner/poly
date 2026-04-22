# vhs tape: record a demo GIF of the poly TUI.
#
# Install: https://github.com/charmbracelet/vhs
# Run:     vhs docs/demo.tape
# Output:  docs/demo.gif
#
# Adjust Sleep durations to taste. Each Type/Enter/Sleep is a "beat" in the
# recording. Requires a configured `poly` (run `poly setup` first) — the GIF
# will show whatever markets / positions / balance your wallet actually has.

Output docs/demo.gif

Set FontSize 14
Set Width 1200
Set Height 720
Set Theme "Dracula"
Set Padding 20
Set TypingSpeed 40ms

# Launch the TUI.
Type "poly"
Enter
Sleep 2s

# Markets tab: search, sort, starred view.
Type "/bitcoin"
Sleep 800ms
Enter
Sleep 1500ms
Escape
Sleep 500ms
Type "s"
Sleep 800ms
Type "s"
Sleep 800ms

# Open market detail, then back out.
Enter
Sleep 2s
Escape
Sleep 500ms

# Cycle through tabs.
Type "2"
Sleep 2s
Type "3"
Sleep 2s
Type "4"
Sleep 2s
Type "5"
Sleep 1500ms

# Help overlay.
Type "?"
Sleep 2s
Escape

# Quit.
Type "q"
Sleep 500ms
Type "y"
Sleep 500ms
