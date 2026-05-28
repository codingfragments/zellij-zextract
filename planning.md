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
| TUI framework | `ratatui` — widgets + `Buffer`, **no Backend**, custom ANSI emitter (crossterm does not compile for `wasm32-wasip1`; verified by Phase 0 Spike B) |
| Crate type | **`[[bin]]`**, not `cdylib` — only binary crates emit the `_start`/`__main_void` WASI reactor exports Zellij's plugin host requires (verified by Phase 0 scaffolding) |
| Toolchain pin | `zellij-tile` minor version must match the running Zellij minor version — plugin ABI is not stable across minors |
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
| Scrollback grab | Named profiles in config (Phase 7); `Ctrl-g` cycles through them at runtime (Phase 8). Defaults: `quick` (last 150) → `deep` (last 1500) → `viewport` → `full`. Optionally bindable to direct-jump keys |
| Snapshot tests | `insta` |
| Logging | `eprintln!` to stderr, no external crate |

## Phase 0 — Tier-0 spikes (GATING)

**Status:** merged (PR #1). Spike B PASSED, Spike A loads cleanly with
API surface confirmed; the behavioral matrix (multi-line safety, SSH
round-trip, special-char preservation) is still an operator
walk-through and feeds Phase 4's insert-action scope.
See `spike-report.md` for the findings and the five scaffolding lessons.

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
- Cross-pattern overlap resolution (Q25 + Phase 4 update): two-pass
  dedup. **Pass 1** collapses same `(type, raw)` keeping the latest
  occurrence. **Pass 2** collapses same `raw` across types, keeping
  the one whose type ranks earliest in a single ordered priority list
  (`extract::TYPE_PRIORITY`); ties resolved by recency. This same list
  drives the picker-rank score bonus (front of list = positive bonus).
  Phase 7 KDL config exposes the order as user-tweakable.
  Default order (highest first):
    url, diagnostic, file, uuid, sha, ipv4, ipv6, command, secret, quoted-string.
- Type tag colors per the palette (hardcoded for now).
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

## Phase 6 — Inline `#type` filter

**Goal:** Query-driven type filtering.

**In scope:**
- Query parser handles `#type` tokens:
  - `#url` → include filter
  - `#!secret` → exclude filter
  - `##main` → escape (literal `#main` fuzzy token)
  - **`#X` with X as a unique prefix** → resolves to that type
    (`#ur` = url, `#sh` = sha, `#se` = secret, etc.)
  - Ambiguous prefix (multiple tags match) → no filter, literal fuzzy
  - Unknown type name → pass through as literal fuzzy token
- Tokenizer parameterized on a `known_tags: &[&str]` slice (NOT
  hardcoded) so v2 can extend with user-defined custom-pattern type
  names from KDL config.
- Active-filter pills shown in the input strip with the type's palette
  color; excludes shown dim-gray with `-` prefix.
- Filters update live as the query is edited.
- Tab to List mode preserves active filters.

**Deferred to Phase 7** (lands with the KDL config that gives users
keybind authoring control anyway):
- Type-preset launch arguments. Plugin reads `configuration` map
  from Zellij launch event; if `type "url"` set, pre-apply `#url`
  filter. Backspaceable like a typed pill. Lives in Phase 7 because
  it's a config-bound keybind feature and Phase 7 owns the config story.

**Out of scope:**
- Tab-completion of type names in `#` token (deferred to v2).

**Acceptance:**
- Typing `#url install` filters to URLs containing "install".
- Typing `#ur` activates the URL filter mid-typing (unique prefix).
- Typing `#u` shows no pill yet (ambiguous: url/uuid).
- Typing `##main` does NOT filter; searches for "#main".
- Backspacing `#url` to `#u` removes the pill mid-typing.

**Tests:**
- 20 unit tests in `query::tests` covering: exact/prefix matches,
  ambiguous-fallback, unknown-fallback, escapes, multiple filters,
  token-order independence, case-insensitivity, caller-supplied tag
  sets (proves v2 extension path).
- Manual: each filter syntax case against the stress fixture.

## Phase 7 — Config file loading + bootstrap

**Goal:** User-configurable patterns, actions, allow-lists, and UI options
via `~/.config/zellij/zextract.kdl`.

**In scope:**
- KDL parser (`kdl` crate) for `zextract.kdl`.
- Path resolution: hardcoded default `~/.config/zellij/zextract.kdl`,
  overridable via `config_path "..."` directive in the zellij plugin block.
- Config schema covers (matching the spec section "Configuration"):
  - `ui { width height position preview preview_open_width preview_closed_width
     mask_secrets theme {...} }`
    - `preview` — default open/closed state: `off` / `auto` / `always` (Phase 4 hardcoded to `off`)
    - `preview_open_width` — width % when preview pane is open (Phase 4 hardcoded `"90%"`)
    - `preview_closed_width` — width % when preview pane is closed (Phase 4 hardcoded `"70%"`)
    - The Phase 4 pane-grow-on-preview behavior (auto-resize via
      `change_floating_panes_coordinates`) becomes config-driven here:
      both widths and the open/closed default state user-tweakable.
  - `grab { profiles { ... } default_profile "..." }` — **named grab profiles**
    (the existing simple `grab "recent" | "viewport" | "full"` + `recent_lines`
    flat keys collapse into this richer model). User defines as many as
    they want and `Ctrl-g` cycles through them in the picker (Phase 8).
    Each profile has:
      - `name "quick" | "deep" | "viewport" | "full" | ...`
      - `source "scrollback" | "viewport"` (scrollback includes lines above viewport)
      - `lines N` — how many trailing lines to keep (or `0` / unset for unbounded)
    Default config ships e.g. three profiles:
      ```kdl
      grab {
          default_profile "quick"
          profiles {
              quick    { source "scrollback"; lines 150  }
              deep     { source "scrollback"; lines 1500 }
              viewport { source "viewport"               }
              full     { source "scrollback"             }
          }
      }
      ```
      Note: within a profile block, each key must be on its own line or
      separated by `;` — whitespace alone does not separate KDL nodes.
    Plus optional `bind "Ctrl 1" profile "quick"` style direct-jump
    bindings (deferred from v1 if too much surface).
  - **Type-preset launch args** (carried over from Phase 6 deferral):
    plugin reads the `configuration` map at `Load`; if it sees
    `type "url"` or `type "url file"`, prepends the corresponding
    `#type` tokens to `self.query` so the picker opens with the
    filter already active. The existing Phase-6 tokenizer + pill
    rendering handles the rest. Lets users wire dedicated keybinds
    like `bind "u" { LaunchOrFocusPlugin "..." { type "url"; }; }`
    for an Alt-U-style "URLs only" picker.
  - `patterns { url {...} file {...} ... secret { formats {...} entropy_fallback {...} } command { prompts triggers continuation_strip } }`
  - `types { url { actions [...] default ... } ... }`
  - `actions { url { open command "..." } file { edit command "..." reveal command "..." } ... }`
  - `limits { copy insert open command json }`
  - `log_level "info"`
  - `editor_command_prefix "nvim"` (Phase 4 hardcoded fallback when `$EDITOR` is unset)
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
- `Ctrl-g` cycles through configured grab profiles (Phase 7 defines them
  in `grab { profiles { ... } }`; v1 defaults are `quick` → `deep` →
  `viewport` → `full`). Each switch re-runs extraction.
  Current profile shown in the stats strip as e.g. `grab:quick(150)` /
  `grab:deep(1500)` / `grab:viewport` / `grab:full(2847 lines)`.
  Status bar briefly displays match-count delta after cycling
  (`+18 matches`) so user can tell if widening helped.
- Optional Phase-8-or-v2: direct-jump bindings (`Ctrl-1` …) to skip
  straight to a named profile without cycling. Config holds the bind
  table; runtime reads on Ctrl-N press.
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

## Phase 9 — CI + release automation

**Goal:** turn the repo into a redistributable plugin — CI runs tests on
every PR, tagged commits produce versioned releases with downloadable
`.wasm` artifacts and install instructions. After this phase the project
is ready for external users.

**In scope:**

- **`.github/workflows/ci.yml`** — runs on every push and PR:
  - matrix: `cargo check`, `cargo test`, `cargo clippy -- -D warnings`,
    `cargo fmt -- --check`
  - Rust toolchain pinned via `rust-toolchain.toml` (already in repo)
  - Add `wasm32-wasip1` target via `rustup target add`
  - Verify the wasm builds successfully (`cargo build --release --target wasm32-wasip1`)
  - Optional: binary-size check — fail if `.wasm` exceeds the budget (1.5 MB)
  - Cache `~/.cargo` and `target/` per [actions-rs cache] pattern
- **`.github/workflows/release.yml`** — runs on `v*.*.*` tag push:
  - Build the release `.wasm`
  - Strip + LTO already configured in `Cargo.toml` release profile
  - Compute SHA-256 of the artifact
  - Use `softprops/action-gh-release` (or equivalent) to create a GitHub
    release with the `.wasm` and a generated checksums file attached
  - Auto-generate release notes from commits since the previous tag
    (use `cliff` / `git-cliff` or GitHub's auto-notes)
- **`CHANGELOG.md`** — Keep a Changelog format, updated per release.
- **README badges** — CI status, latest release version, license.
- **README install section** — three install paths documented:
  1. Download `.wasm` from the latest GitHub release, drop into
     `~/.config/zellij/plugins/`, reference in keybind.
  2. Build from source: `git clone && just install`.
  3. URL load via Zellij's plugin-by-URL feature (if/when available).
- **Reproducible builds** — pin all dep versions in `Cargo.lock` (done),
  ensure `cargo build --locked` succeeds in CI.
- **Versioning** — semantic versioning, starting at v0.1.0 for the first
  Phase 8 cut. Breaking changes (config schema, action verb names, etc.)
  bump minor in pre-1.0.

**Out of scope:**

- Publishing to crates.io — the plugin is a binary artifact, not a
  reusable library, so a crates.io publish doesn't fit. Source distribution
  via the repo + .wasm via releases covers the practical need.
- A plugin marketplace / Zellij plugin registry — not yet a thing for
  Zellij. Watch upstream; if/when it lands we add a publish step here.
- Cross-platform notes — `.wasm` is platform-independent so the artifact
  itself is one file regardless of OS. Install instructions cover
  macOS/Linux paths.
- Code-signing / notarization — not applicable to wasm.

**Acceptance:**
- Push to a feature branch triggers CI; PR shows green checks.
- Pushing a `v0.1.0` tag produces a GitHub release with:
  - `zextract.wasm` and `zextract.wasm.sha256` attached
  - Auto-generated release notes referencing merged PRs since last tag
- Fresh checkout of the tag + `just install` matches the released binary
  byte-for-byte (`cargo build --locked`).
- README install instructions verified by a non-author follower at least once.

## Phase 10 — Documentation

**Goal:** user-facing reference documentation sufficient for a non-author to
install, configure, and extend zextract without reading source code.

**In scope:**

- **Installation guide** — three paths:
  1. Download `.wasm` from the latest GitHub release + verify SHA-256
  2. Build from source: `git clone && just install`
  3. Zellij URL-load if/when the plugin registry lands upstream
  Covers macOS and Linux paths. Keybind setup in `config.kdl` with
  full copy-paste examples (default picker, type-preset keybinds,
  per-keybind overrides).

- **Built-in types reference** — one section per type:
  - Tag name (used in `#filter`, `types { }`, `actions { }`)
  - What it matches (description + representative examples)
  - Default verb fired by `Enter`
  - Available verbs and which require a source pane
  - Key capture fields (`{url}`, `{file}`, `{line}`, …)
  - Any special matching rules (e.g. file requires at least one `/`)

- **Keybind cheatsheet** — table covering Input mode, List mode,
  universal shortcuts (`Alt-g`, `Ctrl-P`, `Ctrl-Y`, …) and multi-select.

- **Configuration reference** — every `zextract.kdl` key:
  - Section, key name, value type, default, valid range
  - One-line description
  - Example snippet

- **Customization guide** with worked examples:
  - `actions { }` — full template variable reference (`{editor}`,
    `{file}`, `{line}`, `{url}`, `{match}`, `{0}`, `{1}`, `{2}`, …),
    `{line}` separator-stripping behaviour, `default` type fallback
  - `types { }` — overriding verb allow-lists and default verb per type
  - `patterns { }` — custom regex patterns step by step: no groups,
    single group (context prefix), multi-group decomposition; full
    worked examples for JIRA tickets, GitHub PRs, port numbers, git
    branch refs
  - Per-keybind overrides (`type`, `preview`, `grab` in Zellij config)

- **Use cases** — narrative walkthroughs:
  - Open URLs from build output
  - Jump to a diagnostic in the editor
  - Insert a command back to the prompt for review
  - Export a selection of file paths as JSON for scripting
  - Wire a dedicated `Alt-j` keybind for JIRA tickets with a custom
    pattern that expands to the full Jira URL

- **Troubleshooting** — common failure modes with diagnosis steps:
  - "could not find exported function" (ABI mismatch + `just clear-cache`)
  - Config changes not taking effect (async load, reload picker)
  - Too many file matches (slash requirement)
  - `Ctrl-W` does nothing (banner not showing / file already exists)
  - Debug output setup (`log_level "debug"` + log tail command)

- **Limitations** — honest list of what v1 cannot do and why:
  - No mouse events (Zellij API gap)
  - No hint mode (requires Zellij core overlay support)
  - macOS `open` hardcoded for reveal/open (Linux: `xdg-open`)
  - No file-content preview (scrollback context only)
  - No multi-pane grab

**Out of scope:**
- In-code API docs (rustdoc) — the plugin is not a library.
- Video walkthroughs / GIFs — nice to have, not blocking.

**Acceptance:**
- A developer unfamiliar with the codebase can install, configure a
  custom JIRA pattern, and wire a type-preset keybind using only the
  documentation (no source reading required).
- Every config key in `zextract.kdl` has a corresponding entry in the
  reference with a correct default and example.

---

## Phase 11 — v1 cleanup + v2 planning

**Goal:** close out v1 cleanly and produce a written design brief for v2
so the next major cycle starts from agreement, not from scratch.

**In scope:**

### UI cleanup backlog

Work through `ui-cleanup.md` and any items accumulated during Phases 8–10:

- **Grab label redesign** — replace the single-line `[quick]` label with
  a two-line display: line 1 = source type (`scrollback` / `viewport`),
  line 2 = line cap (`150 ln` / `1500 ln` / `full`). Both lines
  horizontally centered in the column; the pair bottom-aligned in the
  3-row input strip. Width computed from the widest cap string.

- **Preview: highlight matched span** — use `m.span` byte offsets to
  bold + underline the matched text within the context lines in the
  preview pane. Already partially implemented; needs wiring for the
  exact intra-line byte range. Works for built-ins; custom patterns
  highlight the regex match position (not the expanded template text).

- **Footer verb hints: cap-exceeded coloring** — when a verb's
  multi-target cap would be exceeded by the current selection count,
  render that verb's key hint in a muted/red style rather than bold,
  signalling to the user that pressing it will be refused.

- **Drop preview on/off suffix** — footer shows `p:preview-on` /
  `p:preview-off`; the `-on`/`-off` is redundant now that preview
  state is visually obvious. Show just `p:preview`.
  File: `main.rs::render_footer`.

- **Pane title override** — use `rename_plugin_pane(id, name)` to set
  the floating pane title to `zextract` (or `zextract — #url` when a
  type filter is active). Check exact function name in zellij-tile 0.44.x.

- **Mouse click on grab label** — blocked on Zellij exposing
  `EventType::Mouse` to plugins. Wire `cycle_grab_profile()` on a
  left-click in the grab-label column when the API becomes available.

- **Status message auto-dismiss** — currently any keypress clears
  transient messages; spec says 3-second auto-dismiss. Wire a
  timestamp on `State.message` and clear in `render()` when elapsed.

- **Secret masking** — `ui { mask_secrets true }` is parsed but not
  wired. Replace secret `display` values with `••••••` in the list
  and preview when enabled.

- **Action failure feedback** — `open` / `edit` dispatch currently
  silently succeeds or fails. Surface `run_command` exit codes as a
  transient banner when non-zero.

### Code cleanup

- Remove remaining `#[allow(dead_code)]` and `#![allow(dead_code)]`
  annotations — each should either be deleted or promoted to a real
  public API with a doc comment.
- Audit and resolve all `// Phase N` TODO comments — mark done or
  file as a v2 issue.
- Tighten the `config::schema` re-export list in `mod.rs`; remove
  `#[allow(unused_imports)]` by actually using or removing each export.
- Add `cargo-llvm-cov` to `justfile` (`just coverage`) and wire an
  optional coverage step in CI. Set a baseline once the insta snapshot
  tests (Phase 3 deliverable) are wired.

### v2 design brief

The following topics need a written design decision before v2 work
starts. Record decisions in `planning.md` under a new "v2 Design
Decisions" section.

**Custom pattern priority and hierarchy**

When built-in and user-defined patterns both match the same raw text,
which wins? The current dedup uses `TYPE_PRIORITY` (a static ordered
list) but custom patterns sit outside it. Four candidate models:

1. **Custom-always-wins** — any user pattern that matches a span beats
   any built-in for that span. Simple to reason about; may suppress
   wanted built-in matches.
2. **Append-at-tail** — custom patterns are appended after all built-ins
   in `TYPE_PRIORITY` order, so built-ins win on overlap. Safe default;
   custom patterns only fire when nothing built-in matched.
3. **User-controlled ordering** — a top-level `priority [url file jira
   diag …]` list in `zextract.kdl` that the user populates explicitly,
   including their custom pattern names. Maximum control; more config
   surface.
4. **First-defined-wins** — patterns are evaluated in KDL declaration
   order; first match for a given span wins. Intuitive for per-file
   configs; fragile across config merges.

Decision needed: pick one model (or a hybrid) and document the
reasoning before implementing v2 custom-pattern dedup.

**Other v2 topics to scope:**

- **Hint mode** — in-place labels next to matches in the source pane.
  Requires Zellij core changes for true overlays (shadow pane or
  decoration API). Watch upstream; design the UX spec now so it's
  ready when the API lands.
- **File-content preview** — open the actual file with `std::fs::read`,
  apply syntax highlighting (via `syntect` or a lightweight ANSI
  highlighter), render in the preview pane. Phase 7's `FullHdAccess`
  permission already granted; need to design the "file not found"
  fallback gracefully.
- **Tab-completion of `#type` tokens** — completing `#ur` → `#url`
  inline in Input mode. Requires a small UI state machine separate
  from the current query string; design carefully to avoid breaking
  the backspace / cursor behaviour.
- **Multi-pane scrollback grab** — scrape all panes in the current
  window, merge and deduplicate. Needs a UI affordance to show which
  pane a match came from (e.g. a `{pane}` field in the match record
  and a dim pane-name prefix in the list row).
- **`{name?}` optional template substitution** — strip surrounding
  separator chars only when explicitly opted in (`{line?}` vs `{line}`).
  Currently handled by the `substitute_opt` separator-stripping
  heuristic; make it opt-in to avoid surprising stripping.
- **Configurable keymap** — expose a `keybinds { }` block in KDL
  that remaps List-mode single-letter verbs. Design: only remap
  user-facing verbs (y/i/o/e/r/p/J/g); never allow remapping Esc
  or Tab (would break mode model).
- **Plugin-provided pane title** — already technically possible with
  `rename_plugin_pane`; promote from UI-cleanup to a v2 first-class
  feature if the title should reflect active filters dynamically
  (e.g. `zextract — 18 urls`).
- **Test coverage reporting** — integrate `cargo-llvm-cov`; report
  per-module line coverage in CI as an informational annotation on PRs.
  Set a minimum threshold once snapshot tests are wired.

**Acceptance:**
- `ui-cleanup.md` has no open items.
- All `// Phase N` comments resolved.
- v2 design brief written and reviewed; each topic has a "decision" or
  "deferred with reason" entry.
- `CHANGELOG.md` updated for any patch releases between v0.1.0 and
  the v2 branch cut.

---

## v2 Design Decisions

Written during Phase 11. Each item is either a **locked decision** (ready
to implement in v2) or a **scoped idea** (direction agreed, detail to be
refined when the work starts). All items are explicitly **v2 scope** —
nothing here touches the v1 release.

---

### LOCKED — Custom pattern priority model

**Decision: append-at-tail by default, opt-in `priority` list for power users.**

Built-in patterns always win on overlap (url > diag > file > … per the
existing `TYPE_PRIORITY` list). Custom patterns are appended at the end
of the priority order — they only surface when no built-in claimed the
same raw text. This is zero-config and predictable for the common case.

Power users who need a custom pattern to outrank a built-in (e.g. a
`jira` pattern that should win over `sha` when the ticket ID looks like
a short hash) add an explicit `priority [jira url file …]` list to
`zextract.kdl`. If `priority` is absent, the built-in order + append
behaviour applies.

```kdl
// Optional — only needed when a custom pattern should beat a built-in.
priority ["jira" "url" "file" "diag" "sha" "ipv4" "ipv6" "cmd" "secret" "quote"]
```

**Why not custom-always-wins:** Silently swallows built-in URL matches
that happen to match a ticket pattern. Surprising for users who add a
broad regex and lose their URL list.

**Why not first-defined-wins:** Fragile when users reorder their config
blocks. Not portable across machines.

---

### LOCKED — `{name?}` optional template substitution

**Decision: keep `substitute_opt` stripping as the default; add `{name!}` for
hard-fail.**

The current heuristic (strip preceding `:` / `+` / space when a field
is empty) works well in practice for the edit-command case and requires
no syntax change. In v2, formalise it:

- `{line}` — strip preceding separator if empty (current behaviour, unchanged)
- `{line!}` — emit empty string and do NOT strip; caller sees `src/main.rs:` (rare)
- `{line?}` — synonym for `{line}` (explicit opt-in for clarity in user configs)

No change to `substitute_opt` internals — just document the semantics
and add `?`/`!` suffix parsing to the substitution loop.

---

### LOCKED — Dynamic pane title reflecting active filter

**Decision: update pane title when a type filter is active.**

Extend the existing `rename_plugin_pane` call to reflect the active
`#type` filter:

- No filter → `zextract` (current behaviour)
- `#url` active → `zextract — url`
- `#url #jira` active → `zextract — url · jira`
- `#!secret` active → `zextract — ¬secret`

Call `rename_plugin_pane` in `refilter()` whenever `parsed_query`
changes. Cost: one extra Zellij IPC per keystroke in Input mode — acceptable.

---

### LOCKED — Action failure feedback

**Decision: surface `run_command` non-zero exit as a 3-second warning banner.**

Currently `open` / `reveal` fire `run_command` and silently drop the
result. In v2, subscribe to `Event::RunCommandResult` and check the exit
code. On non-zero:

```
Warning  open failed (exit 127 — xdg-open not found)    ^X:dismiss
```

Use `BannerKind::Warning` (already exists). 3-second auto-dismiss via
`set_timeout`. Zero exit → no banner.

Note: `edit` inserts a command into the source pane rather than running
it, so failure feedback doesn't apply — the user sees the inserted text
and decides whether to run it.

---

### IDEA — Hint mode  *(v2, blocked on Zellij upstream)*

In-place character labels overlaid next to each match in the source pane
(à la `vimium`, `tmux-fingers`). Press a label sequence → action fires
without opening the picker.

**Blocked on:** Zellij exposing a shadow/decoration pane API or an
`overlay_text` primitive. Watch upstream; design UX spec when the API
lands. The extraction logic and match spans are already in place.

**UX sketch:**
- `Alt-h` → hint mode; labels appear over the scrollback
- Type label chars → match selected; default action fires
- `Esc` → cancel

---

### IDEA — File-content preview  *(v2)*

Read the actual file with `std::fs::read` (via the `/host` preopen
already granted) and render its content in the preview pane instead of
just the scrollback context lines. Apply syntax highlighting via a
lightweight ANSI highlighter (e.g. `syntect` with a small grammar set,
or a regex-based fallback).

**Design notes:**
- Fallback gracefully when the file is not found (show scrollback context
  with a "file not found" header, not an error).
- Cap file read at 4 KB to avoid blowing the wasm memory budget.
- Only activate for `file` and `diag` type matches; other types keep the
  scrollback preview.
- Scroll position (no scroll in v1): v2 should bind `Ctrl-U`/`Ctrl-D`
  to scroll the preview independently of the list.

---

### IDEA — Multi-pane scrollback grab  *(v2)*

Scrape all panes in the current window, merge and deduplicate. Show which
pane a match came from.

**Design notes:**
- New `source "all-panes"` grab source in the profile schema.
- Each `Match` gains an optional `pane_id: Option<u32>` field.
- List row prefix: dim pane name/number when grabbing multiple panes.
- Insert action targets the original source pane (not the match pane) —
  behaviour unchanged.
- Dedup: same raw text from different panes kept (pane-id is part of the
  dedup key); same raw from the same pane deduped as today.

---

### IDEA — Tab-completion of `#type` tokens in Input mode  *(v2)*

Completing `#ur` → `#url` inline as the user types, with a small
completion popover or inline ghost text.

**Design notes:**
- Requires a small completion state machine separate from the main query
  string (current: `#ur` stays in the fuzzy text until it resolves on
  space).
- On `Tab` when the cursor is inside a `#`-prefixed token: cycle through
  unambiguous completions. On `Enter` or `Space`: commit the completion.
- Never break backspace behaviour — backspacing into a committed `#url`
  pill removes the whole token, not char-by-char.
- Completion candidates: the current known-tag set (built-ins + loaded
  custom pattern names).

---

### IDEA — Configurable built-in keymap  *(v2)*

Expose a `keybinds { }` block in KDL that remaps List-mode single-letter
verb keys.

```kdl
keybinds {
    copy    "c"    // default: y
    insert  "p"    // default: i
    open    "o"    // default: o (unchanged)
}
```

**Design constraints:**
- Only remap the user-facing verb keys: `y Y i I o e r p J g Space`.
- `Esc`, `Tab`, `Enter`, `Shift-Enter` are structural — never remappable.
- Collision detection at load time: if two verbs map to the same key,
  emit a parse-error banner and fall back to defaults.
- Universal shortcuts (`Ctrl-P`, `Ctrl-Y`, `Alt-g`, etc.) are a separate
  namespace and not configurable in v2.

---

### IDEA — Test coverage reporting  *(v2, CI)*

Integrate `cargo-llvm-cov` and report per-module line coverage as a CI
annotation on PRs. Set a minimum threshold (suggested: 70% line coverage
on `src/pattern/` and `src/config/`) once snapshot tests are wired.

Add `just coverage` to the justfile.

---

## Appendix A — Default `zextract.kdl` (bootstrap-written)

Generated as a string constant in the binary; written verbatim by `Ctrl-W`.
Fully commented so users have a starting point.

```kdl
// zextract — default config
// All settings shown explicitly so you can tweak in place.
// Reload with: zellij action reload-plugin zextract

ui {
    width "70%"                  // legacy alias — same as preview_closed_width
    height "60%"
    position "center"            // center | top | bottom
    preview "off"                // off | auto | always
    preview_closed_width "70%"   // floating pane width when preview is closed
    preview_open_width "90%"     // floating pane width when preview is open (recentered)
    mask_secrets false           // show secret values in the picker (false = visible)
    editor_command_prefix "nvim" // fallback when $EDITOR is unset
    // theme block omitted — uses built-in palette
}

// Named scrollback-grab profiles. Ctrl-g in the picker cycles through
// these in declaration order. Add or rearrange to suit your workflow.
grab {
    default_profile "quick"      // profile loaded on picker launch
    profiles {
        quick    { source "scrollback"; lines 150  }
        deep     { source "scrollback"; lines 1500 }
        viewport { source "viewport"               }  // just what's on screen
        full     { source "scrollback"             }  // unbounded (caveat: extraction cost)
    }
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
