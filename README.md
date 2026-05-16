# zextract

[![CI](https://github.com/codingfragments/zellij-zextract/actions/workflows/ci.yml/badge.svg)](https://github.com/codingfragments/zellij-zextract/actions/workflows/ci.yml)
[![Latest release](https://img.shields.io/github/v/release/codingfragments/zellij-zextract)](https://github.com/codingfragments/zellij-zextract/releases/latest)

A [Zellij](https://zellij.dev) plugin that extracts typed matches from your focused pane's scrollback and presents them in a fuzzy-filterable picker with type-aware actions.

Fills the gap left by tmux tools like `extrakto`, `fingers`, and `fzf-links` for Zellij users.

---

## What it does

Press a keybind → a floating picker opens → the scrollback of your previous pane is scanned for URLs, file paths, diagnostics, commands, secrets, UUIDs, IPs, and any custom patterns you configure → you fuzzy-filter, pick, and act:

- **open** a URL in the browser
- **edit** a file at the matched line in your editor
- **copy** the match to the clipboard
- **insert** it back into the source pane's prompt
- **export** a selection as JSON

---

## Install

### Requirements

- Zellij 0.44.x (plugin ABI is version-locked — see [CLAUDE.md](CLAUDE.md) for details)

### Option 1 — Download binary (recommended)

Download `zextract.wasm` from the [latest release](https://github.com/codingfragments/zellij-zextract/releases/latest):

```sh
# Verify checksum
sha256sum -c zextract.wasm.sha256

# Install
mkdir -p ~/.config/zellij/plugins
cp zextract.wasm ~/.config/zellij/plugins/
```

### Contributing / development setup

After cloning, install the git pre-push hook so fmt + clippy + tests run before every push:

```sh
sh scripts/install-hooks.sh
```

Or run the checks manually at any time:

```sh
just check   # fmt, clippy, test, wasm build — mirrors CI exactly
```

### Option 2 — Build from source

Requires Rust + `wasm32-wasip1` target:

```sh
rustup target add wasm32-wasip1
git clone https://github.com/codingfragments/zellij-zextract
cd zellij-zextract
just build    # cargo build --release --target wasm32-wasip1
just install  # copies zextract.wasm to ~/.config/zellij/plugins/
```

### Keybind

Add to your `~/.config/zellij/config.kdl` inside a `keybinds { normal { ... } }` block:

```kdl
bind "Alt x" {
    LaunchOrFocusPlugin "file://$HOME/.config/zellij/plugins/zextract.wasm" {
        floating true;
    };
}
```

Reload Zellij. Press `Alt x` to open the picker.

---

## Usage

The picker has two modes — **Input** (default) and **List**. `Tab` switches between them.

### Input mode

Type to fuzzy-filter matches. Special `#` prefix syntax filters by type:

| Query | Effect |
|---|---|
| `#url` | Show only URL matches |
| `#file install` | File matches containing "install" |
| `#!secret` | Exclude secrets |
| `#ji` | Unique prefix — resolves to `#jira` if you have a jira pattern |
| `##main` | Literal `#main` in fuzzy search (escape) |
| `#url #file` | Multiple includes |

### List mode

Navigate with `↑`/`↓`. Single-letter keys fire actions on the highlighted match:

| Key | Action |
|---|---|
| `Enter` | Default action for the type (URL → open, file → edit, etc.) |
| `y` | Copy raw value |
| `Y` | Copy display value |
| `i` | Insert raw into source pane prompt |
| `I` | Insert display value |
| `o` | Open (browser / `open`) |
| `e` | Edit in `$EDITOR` |
| `r` | Reveal in Finder / file manager |
| `p` | Toggle preview pane |
| `J` | Export selection as JSON to clipboard |
| `Space` | Toggle multi-select on current row |
| `g` | Cycle grab profile (quick → deep → viewport → full) |
| `Tab` | Switch to Input mode |
| `Esc` | Close picker |

### Universal shortcuts (both modes)

| Key | Action |
|---|---|
| `Alt-g` | Cycle grab profile |
| `Ctrl-P` | Toggle preview |
| `Ctrl-Y` | Force copy-raw |
| `Ctrl-A` | Select all visible |
| `Ctrl-D` | Clear selection |
| `Shift-Enter` | Force insert-raw |
| `Ctrl-X` | Dismiss banner |

### Multi-select

`Space` marks rows. Verb keys act on all marked rows at once, up to per-verb caps (configurable in `limits { }`). Multi-target edit chains commands with ` && `.

---

## Configuration

On first launch with no config file, the picker shows a banner offering `Ctrl-W` to write a default `~/.config/zellij/zextract.kdl`.

Full example with all sections:

```kdl
log_level "info"   // off | error | warn | info | debug

// ── Actions ────────────────────────────────────────────────────────────────
// Override the command used for edit / open / reveal per type.
// Type tags: url, file, diag, sha, ipv4, ipv6, uuid, quote, cmd, secret
// plus any custom pattern names you define below.
//
// Template variables:
//   {editor}  $EDITOR or "nvim"
//   {file}    matched file path
//   {line}    line number (stripped with surrounding : or + if absent)
//   {url}     matched URL
//   {match}   raw match text (or group 1 if regex has groups)
//   {0}       full regex match, {1} group 1, {2} group 2, …

actions {
    diag    { edit command "hx {file}:{line}" }
    default { edit command "{editor} +{line} {file}" }

    // VSCode example:
    // default { edit command "code -g {file}:{line}" }
}

// ── Custom patterns ────────────────────────────────────────────────────────
// User-defined regex patterns. Each gets its own label in the list
// and is filterable with #name.

patterns {
    // Simple: match PROJ-123, API-456, etc.
    jira {
        regex    "([A-Z]+)-([0-9]+)"
        type     "url"
        template "https://jira.example.com/browse/{1}-{2}"
    }

    // Context-anchored: prefix is required to match, group 1 is the value.
    jira-ticket {
        regex    "New Jira ticket : ([A-Z]+-[0-9]+[A-Z]*)"
        type     "url"
        template "https://jira.example.com/browse/{match}"
    }

    // Multi-group: org, repo, PR number.
    github-pr {
        regex    "github\\.com/([^/\\s]+)/([^/\\s]+)/pull/([0-9]+)"
        type     "url"
        template "https://github.com/{1}/{2}/pull/{3}"
    }

    // Port numbers — no template, raw match is the value.
    port {
        regex ":[0-9]{4,5}\\b"
        type  "url"
    }
}

// ── Types ──────────────────────────────────────────────────────────────────
// Override verb allow-lists and default Enter action per type.

types {
    url  { actions "open" "copy" "insert"; default "open" }
    file { actions "edit" "copy" "insert" }
    // Disable insert for secrets:
    // secret { actions "copy" }
}

// ── Limits ─────────────────────────────────────────────────────────────────
// Maximum matches per multi-target verb dispatch.
// Set to 0 to disable a verb entirely.

limits {
    copy   100
    insert   5
    open    10
    edit     5
    reveal  10
    json   100
}

// ── Grab profiles ──────────────────────────────────────────────────────────
// Named scrollback-depth profiles. g / Alt-g cycles through them.

grab {
    default_profile "quick"
    profiles {
        quick    { source "scrollback"  lines 150  }
        deep     { source "scrollback"  lines 1500 }
        viewport { source "viewport"               }
        full     { source "scrollback"             }
    }
}

// ── UI ─────────────────────────────────────────────────────────────────────

ui {
    preview          "off"    // off | auto | always
    preview_open_width  "90%"
    preview_closed_width "70%"
    mask_secrets     false
}
```

---

## Per-keybind overrides

You can override settings per keybind via the Zellij `configuration` map:

```kdl
// Default picker
bind "Alt x" {
    LaunchOrFocusPlugin "file://$HOME/.config/zellij/plugins/zextract.wasm" {
        floating true;
    };
}

// URL-only picker with preview always open
bind "Alt u" {
    LaunchOrFocusPlugin "file://$HOME/.config/zellij/plugins/zextract.wasm" {
        floating true;
        type    "url";
        preview "on";
    };
}

// File/diagnostic picker
bind "Alt f" {
    LaunchOrFocusPlugin "file://$HOME/.config/zellij/plugins/zextract.wasm" {
        floating true;
        type    "file diag";
        preview "off";
    };
}

// Jira tickets with deep scrollback
bind "Alt j" {
    LaunchOrFocusPlugin "file://$HOME/.config/zellij/plugins/zextract.wasm" {
        floating true;
        type "jira";
        grab "deep";
    };
}
```

Supported keys: `type` (space-separated type tags), `preview` (`on`/`off`/`always`), `grab` (profile name).

---

## Troubleshooting

**"could not find exported function" on launch**

Plugin ABI mismatch. The `zellij-tile` version in `Cargo.toml` must match your running Zellij minor version exactly. Check with `zellij --version`, then update the dependency and rebuild.

After any ABI-affecting change, clear the wasmtime cache:

```sh
just clear-cache
# or manually:
rm -rf ~/Library/Caches/org.Zellij-Contributors.Zellij/
```

**Nothing happens when I press the keybind**

Check that `floating true` is set in the keybind — without it Zellij opens the plugin in a non-floating pane which may not have the right context.

**Config changes not taking effect**

The config loads async on each picker open. If patterns or grab settings don't change, reload Zellij or close and reopen the picker once.

**Too many file matches**

File matches require at least one path separator (`/`). Bare names like `Cargo.toml` don't match — only `./Cargo.toml`, `src/Cargo.toml`, `/etc/hosts` etc.

**Debug output**

Set `log_level "debug"` in `zextract.kdl` and tail the Zellij log:

```sh
tail -f ~/Library/Logs/net.Zellij-Contributors.Zellij/zellij.log | grep zextract
```

---

## Architecture notes

- Single Rust crate, `wasm32-wasip1` target
- Regex: `regex-lite` (no lookaround; ~50 KB binary cost vs `regex`)
- TUI: `ratatui` with a custom ANSI emitter (no crossterm — not available in WASI)
- Fuzzy: `nucleo-matcher` scoring crate only
- Config: hand-rolled KDL-subset parser (~200 LOC, no external crate)
- Plugin ABI: `zellij-tile` — pin minor to match running Zellij

See [planning.md](planning.md) for the full phased build history.
