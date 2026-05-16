# UI cleanup items

Running list of UI/UX inconsistencies, dead indicators, polish opportunities,
and small visual cleanups noticed during development. **Don't act on these
in their originating phase** — they accumulate here and get addressed
together either in Phase 8 (preview + polish + edge cases) or as a focused
follow-up PR.

Format: short bullet per item, dated, with file pointer when known.

## Open

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

- **(2026-05-17) Grab label redesign.** Fixed in Phase 11 — two-line
  label outside the input box: dim source type (`scrollback`/`viewport`)
  on top, bold line cap (`150 ln` / `full`) below.
