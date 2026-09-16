# Plan: Forward unbound terminal chords

SPEC Alignment: aligned — no `SPEC.md`; product text is `docs/REFERENCE.md`. Unbound Cmd/Ctrl chords are dropped today; registered Settings shortcuts stay GUI-owned.

Stay in current workspace.

```text
Key (Cmd/Ctrl held, no Text)
  ├─ Settings shortcut consumed? → GUI action, no PTY
  ├─ widget binding (Ctrl+A–Z, ⌘C/V, arrows…)? → existing bytes
  └─ else → kitty CSI u (Cmd=Super, Ctrl=Ctrl)
```

- [x] 1. egui_term: CSI u for unbound modified keys; Ctrl+A–Z and copy/paste unchanged.
- [x] 2. Tests: Cmd+E → `CSI 101;9 u`; Ctrl+E → `0x05`; copy chord not forwarded; consume removes registered shortcut events.
- [x] 3. Docs + CHANGELOG.

Unresolved: none.
