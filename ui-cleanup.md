# UI cleanup items

Running list of UI/UX inconsistencies, dead indicators, polish opportunities,
and small visual cleanups noticed during development. **Don't act on these
in their originating phase** — they accumulate here and get addressed
together either in Phase 8 (preview + polish + edge cases) or as a focused
follow-up PR.

Format: short bullet per item, dated, with file pointer when known.

## Open

- **(2026-05-16) Preview on/off footer indicator obsolete.** The footer
  shows `p:preview-on` / `p:preview-off` reflecting `state.preview_open`.
  Now that `Ctrl-P` works universally and the preview pane is visually
  unmistakable when open, the on/off label is redundant. The `p` hint
  is enough. Drop the `-on`/`-off` suffix; show just `p:preview`.
  File: `crates/zextract/src/main.rs::render_footer`.

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

(none yet)
