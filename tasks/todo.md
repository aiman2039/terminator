# Plan: notification sounds + Winamp-style player + radio

Approved. SPEC.md absent — this file is spec until a human adds one.

## Summary

OS banner sound toggle. One GUI-only Winamp-style player per project (files + radio). Audio dies with GUI. Blitz stays HTML-only.

```
T1 sound ──┐
T2 layout ─┼─ T3 engine ─ T4 UI ─┬─ T5 open files
                                 └─ T6 radio
                                      T7 docs last
```

- [x] Task 1: Notification sound setting (`notification-sound-v1`)
- [x] Task 2: Layout v5 `Tab::Player`
- [x] Task 3: rodio engine worker
- [x] Task 4: Winamp-style UI
- [x] Task 5: Open audio files into player
- [x] Task 6: Radio (Icecast/Shoutcast HTTP(S))
- [x] Task 7: Docs + AGENTS.md

## Accept

- Sound off → silent OS banners; on + unfocused + os_events → system sound
- Click mp3 → player tab, no Neovim; Open as text still editor
- Radio play/stop; bad URL fails closed
- GUI exit / player tab close stops audio
- `cargo fmt/clippy/test`; `cargo xtask gui player` after T4
