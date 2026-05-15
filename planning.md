# zextract — Implementation Plan

A Zellij WASM plugin that extracts typed matches from the focused pane's
scrollback and presents them in a floating fuzzy-picker with type-aware
actions. Replaces the tmux-ecosystem gap of fingers/extrakto/fzf-links.

This plan turns the design pinned in 29 grilling rounds into a phased,
vertically-sliced build order. Each phase ships a runnable, testable plugin —
no horizontal "build the engine, then build the UI" layering. The output of
every phase is something a human can launch, exercise, and break.

## Source-of-truth specs

The canonical design docs live OUTSIDE this repo:

- `~/.config/zellij/fuzzy_finder_spec.md` — v1 functional spec
- `~/.config/zellij/extractor_plugin_requirements.md` — broader requirements
- `~/.config/zellij/gap_analysis_2026-05-14.md` — tmux↔zellij gap inventory

Read these first when picking up the work. This plan synthesizes them with
the decisions made during design.

## Locked architectural decisions

| Area | Decision |
|---|---|
| Scope | v1 = fuzzy picker only; hint-mode deferred |
| Language | Rust, target `wasm32-wasip1` |
| Plugin name | `zextract` |
| Repo layout | Single crate, `justfile`, symlink install |
| Regex engine | `regex` crate (no lookaround) |
| Scan strategy | Per-pattern scan, extract-once, fuzzy-filter on keystroke |
| Fuzzy library | `nucleo-matcher` (just the scoring crate, not the engine) |
| TUI framework | `ratatui` (uncharted in Zellij plugins — spike required) |
| Config format | KDL, separate file at `~/.config/zellij/zextract.kdl` |
| UX model | Modal: Input ↔ List, Tab switches; Esc always closes |
| Type filter | Inline `#type` syntax in query; `Ctrl-f` cycle removed |
| Action model | Hybrid: built-in verbs (`copy`, `insert`, `open`, ...) + `command "..."` escape hatch |
| Template syntax | Rust-style `{name}` placeholders |
| Clipboard | Zellij plugin API (`copy_to_clipboard`), not direct OSC52 |
| JSON export | `J` action, always-array shape, flat per-match objects |
| Lifecycle | Plugin loadable eagerly OR lazily; picker pane created per-invocation |
| Default dimensions | 70%×60% centered floating pane |
| Movement keys | Arrow keys only |
| Insert keys | `i` (raw) / `I` (display) in List mode |
| Scrollback grab | Three modes: `recent` (default, capped at 150 lines) → `viewport` → `full`; `Ctrl-g` cycles |
| Snapshot tests | `insta` |
| Logging | `eprintln!` to stderr, no external crate |

## Phase 0 — Tier-0 spikes (GATING)

Two questions must be answered with working code before locking the spec
further. Both can fail. If either fails, this plan changes substantially.

### Spike A — `write_chars_to_pane_id` from plugin to sibling pane

The insert-back killer feature depends on this. Concrete throwaway plugin:

- Bound keybind opens the plugin in a floating pane.
- Plugin records the previously-focused pane id.
- On `Enter`, plugin calls `write_chars_to_pane_id(text, source_pane_id)`
  with a test payload.
- Plugin closes; we observe the source pane.

Test matrix:

| Scenario | Text payload | Expected |
|---|---|---|
| Local fish shell | `echo hello world` | Text appears at prompt, no auto-execute |
| Local fish shell over SSH | `echo hello world` | Same — bytes round-trip through SSH session |
| TUI app in pane (htop) | `echo hello` | Bytes received by htop (we just want no crash) |
| Special chars | `echo '"$(){}\`' | Literal chars preserved |
| Multi-line via `\n` | `echo a\necho b` | Two lines on prompt, NOT executed |
| Empty | `""` | No-op, no crash |

**Pass criteria:** local-shell + SSH + special-chars all work; bytes land
at the shell's line editor as user input (not auto-executed). If
auto-execute occurs on any payload, document and adjust the spec
(may need to strip trailing newlines from inserts).

**Fail mitigation:** If `write_chars_to_pane_id` doesn't deliver, the
plugin still ships value as a clipboard-only picker (matches → clipboard
via Zellij API). Insert action becomes a no-op. UX is much less interesting
but still better than `EditScrollback` alone.

### Spike B — `ratatui` rendering inside a Zellij WASI plugin

No existing Zellij plugin uses ratatui (all hand-roll ANSI). Throwaway plugin:

- Plugin opens a floating pane.
- Renders a basic ratatui layout: header (input mock), centered list of
  hard-coded rows, footer (action hints mock).
- Responds to `↑`/`↓` to move selection cursor (state update + redraw).
- Responds to Zellij's resize event by recomputing layout.
- Esc closes.

**Pass criteria:**
- Renders without panics, correct widths and column alignment.
- Unicode glyphs (`▸`, `●`, `❯`, `⚠`) render at correct column positions
  (this is the `unicode-width` integration test).
- Resize handler reflows cleanly, no garbage.
- Binary size acceptable (< 1.5 MB stripped + LTO + opt-level=z).

**Fail mitigation:** Fall back to hand-rolled ANSI with a small `Frame`
abstraction. Roughly the same scope, +300 LOC of layout code, -100 KB
binary size. The UI design is identical; only the implementation crate
changes.

### Phase 0 exit

A `spike-report.md` in the repo summarizing pass/fail for each scenario,
plus any spec amendments needed. Spike code can live in `examples/` and
gets deleted (or kept in a `spike/` branch) once Phase 1 starts.

## Phase 1 — Bare end-to-end loop

**Goal:** Launch the plugin, see real URL matches from real scrollback,
copy one to clipboard, close.

**In scope:**
- Cargo workspace skeleton (single crate `zextract`).
- `justfile` with `build`, `install` (symlinks `.wasm` into `~/.config/zellij/plugins/`), `dev` (build + reload).
- `register_plugin!` with cheap `Load` event handler.
- Bound keybind opens floating pane.
- Records source pane id on launch.
- Calls `get_pane_scrollback()` and **takes only the last 150 lines** (hardcoded default
  for this phase; the cap and mode-cycling come from config in Phase 7 and UI in Phase 8).
- Runs ONLY the URL pattern over the captured text.
- Ratatui (or fallback) renders a static list of matches with type tag `[url]`.
- `↑`/`↓` move selection cursor.
- `Enter` copies highlighted match to clipboard via `copy_to_clipboard()`.
- `Esc` closes the plugin pane; Zellij refocuses source pane.

**Out of scope:**
- Input mode / fuzzy filter (matches always shown in extraction order).
- Other patterns.
- Multi-select.
- Action keys beyond `Enter`.
- Config file (everything hardcoded).
- Banners, status bar, footer hints.
- Bottom bar dynamic hints.

**Acceptance:**
- `cat tests/fixtures/urls.txt` in a pane → press keybind → see URLs in the
  picker → arrow to one → Enter → URL is on clipboard.
- Plugin pane closes cleanly, source pane regains focus.
- WASM binary ≤ 1.5 MB stripped+LTO.
- Cold launch < 100 ms when eagerly loaded.

**Tests:**
- Unit test for URL pattern regex against a small inline string.
- Manual smoke test against `tests/fixtures/urls.txt`.

## Phase 2 — Fuzzy filter + Input mode

**Goal:** Real-time fuzzy filtering with smart-case, ranked output.

**In scope:**
- Add input line at top of picker.
- Plain typing edits the query.
- `nucleo-matcher` scores all matches against the query on every keystroke.
- Smart-case (uppercase in query → case-sensitive).
- Selection cursor jumps to top of filtered list if previously-selected
  row is filtered out; otherwise stays.
- Match-character highlighting in row text using nucleo's returned indices
  (bold the matched chars).
- `Backspace` edits query.
- Still single-mode (no Tab to List yet).
- Still only URL pattern.

**Out of scope:**
- Other patterns.
- `#type` inline filter syntax.
- Type-priority scoring bonuses (apply only fuzzy score; bonuses added Phase 3).
- Modal switch.

**Acceptance:**
- Typing in the picker narrows the list live.
- Smart-case works: `EXample` filters case-sensitively; `example` doesn't.
- Matched characters are visually highlighted in each row.
- Selection cursor behavior is sane across filter changes.

**Tests:**
- Unit test for the fuzzy filter wrapper against canned input + queries.
- Manual smoke test: filter URLs with various queries.

## Phase 3 — Full pattern set + snapshot tests

**Goal:** All v1 patterns extract correctly with regression coverage.

**In scope:**
- Implement all default patterns:
  - `url`, `file` (+`:line[:col]`), `diagnostic`, `sha`, `ipv4`, `ipv6`,
    `uuid`, `quoted-string`, `command` (hybrid prompt + exec-anchored,
    with continuation splicing + prefix stripping per Q12-Q13),
    `secret` (curated formats: JWT, AWS, GitHub, GitLab, Stripe, OpenAI,
    Anthropic, Slack, Bearer + entropy fallback per Q14-Q15).
- Cross-pattern overlap resolution (Q25): emit all cross-type matches,
  dedupe same-type-same-raw keeping latest occurrence, leftmost-longest
  within a single pattern.
- Type tag colors per the palette (hardcoded for now).
- Type-priority scoring bonuses applied to nucleo score.
- Per-type capture fields populated into the `Match.fields` map
  (Q20: `{url}`/`{scheme}`/`{host}` via `url` crate, `{file}`/`{line}`/
  `{col}`/`{dir}`/`{basename}`/`{ext}` via `std::path::Path`, etc.).
- `tests/fixtures/*.txt` files: one per pattern type + `realworld.txt` +
  `adversarial.txt` per Q10.
- `insta` snapshot tests in `tests/extract.rs` asserting extracted
  `(type, raw, display)` triples per fixture.

**Out of scope:**
- Modal flow (still single-mode).
- Actions beyond copy via Enter.
- Inline `#type` filter (next phase).

**Acceptance:**
- `cargo test` passes; every fixture has a corresponding snapshot.
- `cargo insta review` shows clean snapshot state.
- Manual: cat each fixture file, launch picker, visually verify the
  expected types appear with correct tags.
- Performance: extraction on 10,000-line scrollback completes < 50 ms.

**Tests:**
- Per-pattern unit tests for boundary cases (empty input, single match,
  overlapping captures).
- Integration test sweeping all fixtures via `insta`.

## Phase 4 — Modal flow + full action layer

**Goal:** Input ↔ List mode, all built-in action verbs working, type-aware
defaults and allow-lists, source-pane insert via Phase 0 spike A.

**In scope:**
- Tab toggles Input ↔ List mode.
- Visual mode indicator (`[INPUT]` / `[LIST]` tag, input bg brightness).
- Plain letter keys in List mode are actions:
  - `y` copy raw / `Y` copy display
  - `o` open (type-defined)
  - `e` edit (file/diagnostic only)
  - `r` reveal (file only)
  - `i` insert raw / `I` insert display
  - `p` preview toggle (stubbed — actual preview in Phase 8)
- Per-type allow-list with `copy` always implicitly allowed.
- Default action per type (Enter): URL→open, file→edit, diagnostic→edit,
  command→insert, secret→copy, sha/uuid/ipv4→copy.
- `command "..."` action verb with `{name}` template substitution.
- Insert dispatch via `write_chars_to_pane_id` (requires Phase 0 spike A pass).
- Open dispatch via `RunCommand` for `open`/`edit`/`reveal`.
- Dynamic bottom-bar footer showing keys valid for current row's type.
- Silent-reject of disallowed keys in List mode.
- Hardcoded type-action map (config file lands in Phase 7).

**Out of scope:**
- Multi-select (next phase).
- `J` JSON export (next phase).
- `Ctrl-g` grab-area toggle (defer to polish).
- KDL-configured per-type overrides (Phase 7).

**Acceptance:**
- For each type in the default set: every action key fires its expected
  effect; disallowed keys silently no-op.
- Insert works locally and over SSH (Phase 0 spike test re-run).
- Open URL → browser launches.
- Open file → `$EDITOR` launches at line.
- Mode switch via Tab is visually obvious and immediate.

**Tests:**
- Unit tests for template substitution (universal vars + each per-type set).
- Manual test sheet: for each (type, action) cell, fire and verify.

## Phase 5 — Multi-select + JSON export

**Goal:** Power-user multi-row workflows.

**In scope:**
- `Space` toggles selection on current row (List mode only).
- `Ctrl-a` selects all visible (post-filter).
- `Ctrl-d` deselects all.
- Selection persists across query changes; cleared on picker close.
- Selection count in header: `N selected · M matches`.
- Selected-row visual: leftmost `●` + lighter background.
- Per-action caps (configurable; defaults per Q24:
  copy=100, insert=5, open=10, command=5, json=100).
- Refuse-loudly when cap exceeded (status bar message, picker stays open).
- Join semantics per Q24:
  - copy: `\n`-joined
  - insert: ` `-joined (safety: avoid accidental shell exec)
  - edit: as separate args to `$EDITOR`
  - reveal: per-file invocation, capped
  - open: per-URL invocation, capped
- Mixed-type firing: silent-permissive; loud-reject only if zero rows valid.
- Enter on mixed selection: each row's own default fires in parallel.
- `J` action: JSON export to clipboard.
  - Always-array shape (single-row case still `[{...}]`).
  - Flat per-match objects (universal fields + per-type fields at same level).
  - All values stringified.
  - Compact single-line output.
  - Always allowed (like `y`), regardless of allow-list.

**Out of scope:**
- Inline `#type` filter (next phase).
- Config-driven cap overrides (Phase 7 wires KDL).

**Acceptance:**
- Multi-select 3 URLs → `o` → 3 browser tabs.
- Multi-select 5 file paths → `e` → editor opens with all 5 as args.
- Multi-select 11 files → `o` → cap-exceeded message, picker stays open.
- Mixed: 2 URLs + 1 secret → `o` → 2 tabs open, secret silently skipped.
- `J` on single match → array of 1 in clipboard, valid JSON.
- `J` on multi-select → array of N, valid JSON.

**Tests:**
- Unit tests for JSON shape generation.
- Unit tests for cap enforcement.
- Manual: every multi-select × action combination from the matrix.

## Phase 6 — Inline `#type` filter + type-preset launch args

**Goal:** Query-driven type filtering and per-type keybindings.

**In scope:**
- Query parser handles `#type` tokens:
  - `#url` → include filter
  - `#!secret` → exclude filter
  - `##main` → escape (literal `#main` fuzzy token)
  - Unknown type name → pass through as literal fuzzy token
- Active-filter pills shown in the stats strip above the list.
- Filters update live as the query is edited.
- Tab to List mode preserves active filters.
- Type-preset launch arguments per Q27:
  - Plugin reads `configuration` map from Zellij launch event.
  - If `type "url"` set → pre-apply `#url` filter.
  - If `type "url file"` set → pre-apply both.
  - Backspaceable from Input mode if user wants to broaden.

**Out of scope:**
- Tab-completion of type names in `#` token (deferred to v2).

**Acceptance:**
- Typing `#url install` filters to URLs containing "install".
- Typing `##main` does NOT filter; searches for "#main".
- `LaunchOrFocusPlugin "zextract" { type "url" }` opens with URL filter
  already active and a pill visible.
- Removing a pill via backspace re-broadens the list immediately.

**Tests:**
- Unit tests for the query parser (sigil handling, escape, negation,
  unknown-type passthrough).
- Manual: each filter syntax case + preset bindings.

## Phase 7 — Config file loading + bootstrap

**Goal:** User-configurable patterns, actions, allow-lists, and UI options
via `~/.config/zellij/zextract.kdl`.

**In scope:**
- KDL parser (`kdl` crate) for `zextract.kdl`.
- Path resolution: hardcoded default `~/.config/zellij/zextract.kdl`,
  overridable via `config_path "..."` directive in the zellij plugin block.
- Config schema covers (matching the spec section "Configuration"):
  - `ui { width height position preview mask_secrets grab recent_lines theme {...} }`
  - `patterns { url {...} file {...} ... secret { formats {...} entropy_fallback {...} } command { prompts triggers continuation_strip } }`
  - `types { url { actions [...] default ... } ... }`
  - `actions { url { open command "..." } file { edit command "..." reveal command "..." } ... }`
  - `limits { copy insert open command json }`
  - `log_level "info"`
- User-extensible patterns: any `patterns.<name>` block with `regex`,
  `type`, and optional `template "..."` becomes a new pattern.
- Custom action verbs only via `command "..."` template.
- Hardcoded denies that override config:
  - `secret` type cannot list `open` or `run` in its `actions [...]` allow-list
    (config error on load; warn loudly).
  - Custom patterns cannot define field names colliding with universal
    field names (`type`, `raw`, `display`, `context`, `span`).
- Config re-read on every plugin launch (no polling; manual reload via
  `zellij action reload-plugin zextract` for dev).
- WASI filesystem read permission requested at plugin init.
- Bootstrap UX per Q10:
  - Missing config → yellow banner: "No config at ~/.config/zellij/zextract.kdl —
    using built-in defaults. [Ctrl-W] write defaults · [Ctrl-X] dismiss & don't
    show again".
  - `Ctrl-W` requests FS write permission (on-demand, not at init), writes
    a fully-commented default config.
  - `Ctrl-X` writes `~/.config/zellij/.zextract-dismissed` marker.
  - Parse failure → red banner with line/col + error; NO overwrite key
    (preserve user data); manual fix only.
- Default config content baked as a string constant (Appendix A).

**Out of scope:**
- Remappable built-in keys (v2).
- Live config watching / inotify (v2 if WASI ever gets it).
- Multi-config-file include directives (v2).

**Acceptance:**
- Fresh user (no config) launches plugin → banner appears → Ctrl-W →
  permission prompt → config written → next launch loads from file.
- Edit `zextract.kdl` to add a custom Jira pattern + open action →
  reload → Jira ticket in scrollback shows as match → Enter opens browser.
- Introduce a typo → reload → parse-error banner with line/col, defaults
  used, no clobbering.
- Try setting `secret { actions ["open" "copy"] }` → config-load error
  visible.

**Tests:**
- Unit tests for KDL parser per config section.
- Unit tests for the hardcoded-deny rules.
- Snapshot test that the bootstrap-written default config round-trips
  through the parser cleanly.

## Phase 8 — Preview, polish, edge cases

**Goal:** Production-ready quality. Everything left over.

**In scope:**
- Preview pane per Q28:
  - Context-only (5 lines above + 5 below the match's line from captured scrollback).
  - `p` toggles open/closed; default off.
  - `ui { preview "off" }` default in bootstrap; `auto` and `always` honored.
  - No scrolling in v1.
- Banner system per Q29:
  - Yellow for warnings (bootstrap missing, source pane disappeared mid-flow).
  - Red for errors (config parse failed).
  - Dismissible.
- Status-bar transient messages per Q29:
  - Action failures (`Open failed: xdg-open exited 127. See log.`).
  - Cap exceeded.
  - 3-second timeout or any-key dismiss.
- Empty states per Q29:
  - "No matches in pane scrollback" with `Ctrl-g` suggestion when grab=viewport.
  - "No matches for <query>" when filter empties the list.
- `Ctrl-g` cycles grab area: `recent (capped)` → `viewport` → `full` → `recent`; re-runs extraction on each switch.
  Current mode shown in the stats strip as e.g. `grab:recent(150)` / `grab:viewport` / `grab:full(2847 lines)`.
  Status bar briefly displays match-count delta after cycling (`+18 matches`) so user can tell if widening helped.
- Truncation per Q26: middle-truncate for url/file, end-truncate for others.
- Minimum-size guard: render "terminal too small" at <60×15.
- Edge cases:
  - Source pane disappears mid-flow: banner + disable insert/open, retain copy/J.
  - Re-invocation while picker open: focus existing (LaunchOrFocus does this for free).
  - Plugin as source pane: documented limitation; no special handling.
- Logging polish:
  - INFO line on launch with extraction stats (count + ms + per-type breakdown).
  - WARN on config issues.
  - ERROR on action failures.
- Documentation:
  - `README.md` with: install (eager + lazy patterns), keybind examples
    (default, URL-preset, file-preset), config file format pointer,
    troubleshooting.
  - `CHANGELOG.md` started.

**Out of scope (deferred to v2):**
- Hint mode (shadow-pane in-place labels).
- Tab-completion of `#type` tokens.
- File-content preview (actual file I/O, syntax highlighting).
- Configurable built-in keymap.
- `{name?}` optional template substitution.
- Pattern priority tiers within a single type.
- Multi-pane scrollback grab ("all panes in window").

**Acceptance:**
- Full feature set from spec is exercised by a manual QA pass.
- All snapshots green; no clippy warnings; no test flakes.
- Cold launch < 100 ms; first-launch (lazy) < 250 ms.
- WASM binary ≤ 1.5 MB.

## Appendix A — Default `zextract.kdl` (bootstrap-written)

Generated as a string constant in the binary; written verbatim by `Ctrl-W`.
Fully commented so users have a starting point.

```kdl
// zextract — default config
// All settings shown explicitly so you can tweak in place.
// Reload with: zellij action reload-plugin zextract

ui {
    width "70%"
    height "60%"
    position "center"      // center | top | bottom
    preview "off"          // off | auto | always
    mask_secrets false     // show secret values in the picker (false = visible)
    grab "recent"          // recent | viewport | full ; Ctrl-g in the picker cycles
    recent_lines 150       // when grab="recent", scan only the last N lines of scrollback
    // theme block omitted — uses built-in palette
}

log_level "info"           // off | error | warn | info | debug

limits {
    copy 100
    insert 5
    open 10
    command 5
    json 100
}

patterns {
    url     { enabled true }
    file    { enabled true }
    diagnostic { enabled true }
    sha     { enabled true  min_len 7 }
    ipv4    { enabled true }
    ipv6    { enabled false }   // off — rare; flip to enable
    uuid    { enabled true }
    quoted_string { enabled true }
    hex     { enabled false min_len 4 }
    number  { enabled false min_len 4 }

    secret {
        enabled true
        // Curated formats:
        formats { jwt; aws; github; gitlab; stripe; openai; anthropic; slack; bearer }
        entropy_fallback {
            enabled true
            min_length 20
            max_length 200
            min_class_count 3
            min_entropy_bits 3.5
        }
    }

    command {
        enabled true
        prompts ["❯ " "$ " "> " "% " "# "]
        triggers [
            "sudo" "apt" "apt-get" "yum" "dnf" "pacman" "brew" "snap"
            "pip" "pip3" "pipx" "gem" "cargo" "go" "npm" "yarn" "pnpm"
            "bun" "uv" "poetry" "conda" "mamba"
            "curl" "wget" "fetch"
            "sh" "bash" "zsh" "fish" "/bin/sh" "/bin/bash"
            "make" "cmake" "ninja" "just" "nix" "nix-shell" "nix-build"
            "nvim" "vim" "nano" "emacs" "less" "more" "cat" "tee"
            "xargs" "awk" "sed" "grep" "find"
            "git" "hg" "svn"
            "docker" "podman" "kubectl" "helm"
            "python" "python3" "node" "deno" "ruby" "rustc" "java" "mvn" "gradle"
            "tar" "gunzip" "unzip" "chmod" "chown" "ln" "mkdir" "rm" "cp" "mv"
            "ssh" "scp" "rsync"
        ]
        continuation_strip [
            "^\\s*\\d+[:\\.]?\\s+"     // line numbers
            "^[+\\-]\\s+"              // diff markers
            "^[#>|]\\s+"               // comments / quotes
            "^\\s+"                    // leading whitespace
        ]
        max_continuation_lines 10
    }
}

types {
    url {
        actions ["open" "copy" "insert"]
        default "open"
    }
    file {
        actions ["edit" "open" "reveal" "copy" "insert"]
        default "edit"
    }
    diagnostic {
        actions ["edit" "open" "copy" "insert"]
        default "edit"
    }
    sha {
        actions ["copy" "insert"]
        default "copy"
    }
    ipv4 {
        actions ["copy" "insert"]
        default "copy"
    }
    ipv6 {
        actions ["copy" "insert"]
        default "copy"
    }
    uuid {
        actions ["copy" "insert"]
        default "copy"
    }
    quoted_string {
        actions ["copy" "insert"]
        default "copy"
    }
    command {
        actions ["insert" "copy" "edit-run"]
        default "insert"
    }
    secret {
        actions ["copy" "insert"]     // no open/run allowed for secrets
        default "copy"
    }
}

actions {
    url {
        open command "open {url}"               // macOS; use "xdg-open {url}" on Linux
    }
    file {
        edit   command "$EDITOR {file} +{line}"
        open   command "open {file}"
        reveal command "open -R {file}"
    }
    diagnostic {
        edit command "$EDITOR {file} +{line}"
        open command "open {file}"
    }
}
```

## Appendix B — Test fixture inventory

```
tests/
├── fixtures/
│   ├── urls.txt              # curl/wget output, OAuth redirect chains
│   ├── files.txt             # rg, ls -la, git status output
│   ├── filepaths_lineno.txt  # cargo error, eslint, "file:42:8" style
│   ├── git_log.txt           # `git log --oneline` + bare SHAs in commit msgs
│   ├── diagnostics.txt       # rustc errors, python tracebacks
│   ├── commands.txt          # shell-history-style transcript
│   ├── commands_multiline.txt # commands with trailing-\ continuations and prefix noise
│   ├── secrets.txt           # synthetic API keys, JWTs, bearer tokens
│   ├── kube.txt              # kubectl get pods/svc output (off by default)
│   ├── realworld.txt         # mixed dev-session transcript
│   └── adversarial.txt       # near-misses, false positives
├── extract.rs                # integration: read fixture → extract → snapshot
└── snapshots/                # insta-managed
```

All synthetic content. NEVER check in real secrets.

## Appendix C — Known v1 limitations

Documented in `README.md` rather than fixed:

- Hint mode (in-place labels next to matches in original pane) — not
  achievable without Zellij core changes. Use scrollback editor (`Ctrl-s e`)
  for the cases that need spatial preservation.
- Cross-pane / whole-window scraping — focused pane only.
- ANSI styling in captured scrollback is dropped by Zellij's API; captures
  are plain text.
- Plugin-to-plugin pipelines.
- Pasting a large payload into the source pane mid-extraction: extraction
  sees only the bytes present at launch time. Re-invoke after paste settles.
- Plugin pane being the source pane (zextract invoked from inside another
  plugin's pane): `write_chars_to_pane_id` will deliver bytes to that
  plugin's input handler, which may or may not handle them gracefully.
- Configurable built-in keymap — fixed in v1.
- File-content preview — context-only in v1.

## References

- Zellij plugin API: https://zellij.dev/documentation/plugin-api-commands.html
- ratatui: https://github.com/ratatui-org/ratatui
- nucleo-matcher: https://crates.io/crates/nucleo-matcher
- regex: https://docs.rs/regex
- kdl: https://github.com/kdl-org/kdl-rs
- insta: https://insta.rs
- tmux-fingers (reference): https://github.com/Morantron/tmux-fingers
- extrakto (reference): https://github.com/laktak/extrakto
- tmux-fzf-links (reference): https://github.com/alberti42/tmux-fzf-links
