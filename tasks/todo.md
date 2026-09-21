# Plan: fatal panic dump + dialog

SPEC Alignment: misaligned — SPEC.md absent; this file is spec until a human adds one.

Approved in session. Stay here. AGENTS.md snippet allowed. Path-only dialog.

```
panic → owner thread? → write $data/crashes/*.log → GUI dialog? → stderr → exit
              └ no → stderr only (workers already catch_unwind)
```

- [x] T1 core `crash.rs`: install, owner-thread dump, prune 20, tests
- [x] T2 GUI: hook + NSAlert/rfd, skip in CI/fixtures
- [x] T3 daemon + hook: dump, no dialog
- [x] T4 docs + AGENTS.md

No resume. No minidumps. No `human-panic`. No `catch_unwind` around eframe.
