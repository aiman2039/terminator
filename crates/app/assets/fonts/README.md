# Bundled static fonts

Unmodified TTFs and licenses extracted from official releases:
- Inter 4.1: https://github.com/rsms/inter/releases/tag/v4.1 (`extras/ttf`).
- JetBrains Mono 2.304: https://github.com/JetBrains/JetBrainsMono/releases/tag/v2.304 (`fonts/ttf`).
- Noto Sans Symbols 2 (braille subset U+2800–U+28FF): https://github.com/notofonts/notofonts.github.io (`fonts/NotoSansSymbols2/hinted/ttf`). Appended as a monospace fallback so TUIs that draw logos in Braille Patterns render instead of tofu.
- Noto Sans Hebrew Regular: https://github.com/notofonts/notofonts.github.io/blob/main/fonts/NotoSansHebrew/hinted/ttf/NotoSansHebrew-Regular.ttf. Unmodified static font, appended to normal/bold terminal and UI families for Hebrew glyph coverage. License: https://github.com/notofonts/hebrew/blob/main/OFL.txt (bundled as `NotoSansHebrew-OFL.txt`). Downloaded 2026-09-14; SHA-256 `cdefaf8efd47045f6820928eba84db5bed7557539328952b5f828315485e02ee`. This supplies glyphs, not terminal bidirectional reordering.

Typography reference: https://plugins.jetbrains.com/docs/intellij/typography.html

Archive SHA-256:
- `/tmp/inter.zip`: `9883fdd4a49d4fb66bd8177ba6625ef9a64aa45899767dde3d36aa425756b11e`
- `/tmp/jb.zip`: `6f6376c6ed2960ea8a963cd7387ec9d76e3f629125bc33d1fdcd7eb7012f7bbf`
