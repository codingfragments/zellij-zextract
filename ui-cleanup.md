# UI cleanup items

Running list of UI/UX inconsistencies, dead indicators, polish opportunities,
and small visual cleanups noticed during development. **Don't act on these
in their originating phase** — they accumulate here and get addressed
together either in Phase 8 (preview + polish + edge cases) or as a focused
follow-up PR.

Format: short bullet per item, dated, with file pointer when known.

## Open

- **(2026-05-17) Command pattern false positives.** The `cmd` pattern
  fires too broadly — executable-anchored matches (trigger list: `sudo`,
  `curl`, `git`, `kubectl`, etc.) produce false positives in prose and
  log output where those words appear without being actual commands.
  Needs a review of the trigger list, tighter anchor rules (e.g. require
  start-of-line or prompt context), and possibly a confidence threshold
  before surfacing as a `cmd` match.
  File: `crates/zextract/src/pattern/command.rs`.

- **(2026-05-16) Mouse click on grab indicator should cycle profiles.**
  The `[quick]` label outside the input box is visible but not clickable.
  zellij-tile 0.44.3 has no `EventType::Mouse` for plugins.
  When Zellij adds plugin mouse events, wire a left-click in the grab
  label column to `cycle_grab_profile()`.

## How to add an item

When you notice a UI nit during a phase that isn't worth interrupting the
current work to fix, append a line under "Open" with:

```
- **(YYYY-MM-DD) Short title.** Description of the issue and why it
  matters. Suggested fix if obvious. File pointer if known.
```

If the fix lands in some later phase, move the item to "Resolved" with
the commit hash that fixed it.

## Resolved

- **(2026-05-16) Preview on/off footer indicator obsolete.** Fixed in
  Phase 11 — footer shows `p:preview`; status message on toggle removed.

- **(2026-05-17) Status message auto-dismiss.** Fixed in Phase 11 —
  `set_message()` arms a 3-second `set_timeout`; `Event::Timer` clears
  it. Keypress still clears immediately too.

