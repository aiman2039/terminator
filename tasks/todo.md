# Plan: in-file unsaved close bar

**Summary:** Dirty close uses a floating `"Close file"` window. Discard unmounts it; any `Err` remounts it. `Stopping` is still live, so the 2s wait can fail and the prompt returns forever. While up, every terminal is unfocused. Replace with a high-visibility bar inside the file. Do not remount. Do not block the rest of the app.

**SPEC Alignment:** aligned — `AGENTS.md` / `docs/REFERENCE.md`: Save / Discard / Cancel; unknown state must not silent-discard. No `SPEC.md`.

```
close file X
  already prompting these ids? -> ignore
  -> Check
       clean -> :qa -> gone
       dirty -> in-file bar
                  Save -> wall+:qa -> close | fail: same bar
                  Discard -> :qa!+Stop -> Ended|Stopping = close | fail: same bar
                  Cancel -> keep
```

## Task 1 — Stop remount / re-queue [done]
- Keep prompt across Save/Discard (do not unmount on click).
- Extra X on same ids: no-op.
- Discard success if `Ended` or `Stopping`. Check/Save never Stop.
- Buttons disabled while quit in flight.

## Task 2 — In-pane bar + unblock focus [done]
- Remove `"Close file"` window.
- Bar under caption/markdown header (incl. Preview): `status_failed` wash, Save / Discard / Cancel.
- Drop global unfocus. Only in-flight quit ids lose PTY focus.
- Keep fixture button labels.

## Task 3 — Docs + fixtures [done]
- REFERENCE / AGENTS: in-file bar, app stays usable.
- `cargo xtask gui file-close` passed, including `unsaved-close-bar` capture.
- Unit: extra close does not queue another Check; failed close updates the same prompt.

## Unresolved
- Shell `"Close tab?"` stays a popup.
