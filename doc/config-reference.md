# Configuration reference

zextract reads `~/.config/zellij/zextract.kdl` on every picker launch.
If the file is missing, defaults are used and a banner offers `Ctrl-W`
to write a starter config.

---

## Top-level scalars

| Key | Type | Default | Description |
|---|---|---|---|
| `log_level` | string | `"info"` | Verbosity of `[zextract]` stderr output. One of `off`, `error`, `warn`, `info`, `debug`. |

---

## `ui { }` block

Controls visual behaviour.

| Key | Type | Default | Description |
|---|---|---|---|
| `preview` | string | `"off"` | Preview pane state at launch. `"off"` = closed, `"auto"` = closed but remembers last state, `"always"` = open. |
| `preview_open_width` | string | `"90%"` | Floating pane width when preview is open. Percent string or pixel count. |
| `preview_closed_width` | string | `"70%"` | Floating pane width when preview is closed. |
| `mask_secrets` | bool | `false` | Replace secret match display values with `••••••` in the list. *(parsed; not yet wired in v0.1.0)* |

**Example:**
```kdl
ui {
    preview              "always"
    preview_open_width   "85%"
    preview_closed_width "65%"
    mask_secrets         true
}
```

---

## `colors { }` block

Overrides the UI color palette. All keys are optional — omit the block
entirely (or any individual key) to keep the built-in defaults. Defaults
reproduce the appearance of versions before `0.5.0` exactly.

### Color value format

| Format | Example | Notes |
|---|---|---|
| ANSI name | `"dark_gray"` | `black`, `dark_gray`, `gray`, `white`, `red`, `green`, `yellow`, `blue`, `magenta`, `cyan`, `light_red`, `light_green`, `light_yellow`, `light_blue`, `light_magenta`, `light_cyan` |
| Hex | `"#rrggbb"` | Six-digit lowercase hex |
| RGB | `"rgb(r,g,b)"` | Decimal 0–255 per channel |

### UI chrome slots

| Key | Default | Used for |
|---|---|---|
| `muted` | `"dark_gray"` | Gutters, hints, secondary text, context lines in preview, empty-state messages |
| `accent` | `"cyan"` | Selected-item `●` bullet; progress bar fill |
| `cursor_bg` | `"blue"` | List cursor row background; input-mode `▍` marker |
| `cursor_fg` | `"black"` | List cursor row foreground — must contrast `cursor_bg` |
| `highlight` | `"yellow"` | Fuzzy-match character highlights, preview match-line `▸`, banner border, warning label, footer status messages |
| `error` | `"light_red"` | Config parse-error label in the banner |
| `fallback_type` | `"gray"` | Color for custom pattern types that have no explicit `type_*` slot |

### Type color slots

Each slot controls the `[tag]` pill in the list and the match highlight in the preview.

| Key | Default | Type tag |
|---|---|---|
| `type_url` | `"blue"` | `url` |
| `type_file` | `"green"` | `file` |
| `type_diag` | `"light_red"` | `diag` |
| `type_git` | `"yellow"` | `git` |
| `type_sha` | `"yellow"` | `sha` |
| `type_ipv4` | `"cyan"` | `ipv4` |
| `type_ipv6` | `"cyan"` | `ipv6` |
| `type_uuid` | `"magenta"` | `uuid` |
| `type_quoted` | `"gray"` | `quote` |
| `type_command` | `"light_magenta"` | `cmd` |
| `type_secret` | `"light_red"` | `secret` |

### Theme presets

Five complete presets are included as commented examples in the bootstrap
config (`Ctrl-W`): **Catppuccin Mocha**, **Catppuccin Macchiato**,
**Catppuccin Latte** (light), **Tokyo Night**, and **Gruvbox Dark**.

**Minimal example — change two slots, keep the rest:**
```kdl
colors {
    cursor_bg  "#7aa2f7"   // Tokyo Night blue
    cursor_fg  "#1a1b26"   // Tokyo Night background
}
```

**Full default listing (ANSI palette):**
```kdl
colors {
    muted          "dark_gray"
    accent         "cyan"
    cursor_bg      "blue"
    cursor_fg      "black"
    highlight      "yellow"
    error          "light_red"
    fallback_type  "gray"
    type_url       "blue"
    type_file      "green"
    type_diag      "light_red"
    type_git       "yellow"
    type_sha       "yellow"
    type_ipv4      "cyan"
    type_ipv6      "cyan"
    type_uuid      "magenta"
    type_quoted    "gray"
    type_command   "light_magenta"
    type_secret    "light_red"
}
```

---

## `grab { }` block

Controls how much scrollback is captured. `g` (List mode) or `Alt-g`
(both modes) cycles through profiles at runtime.

### `default_profile`

Which profile is active on launch. Must match a name in `profiles { }`.
Typos fall back to the first defined profile.

### `profiles { }` block

Each child is a named profile. If `profiles { }` is present, it **replaces**
the four built-in defaults entirely — users who define even one profile
must list all the profiles they want.

**Built-in defaults (used when `profiles { }` is absent):**

| Name | Source | Lines |
|---|---|---|
| `quick` | scrollback | 150 |
| `deep` | scrollback | 1500 |
| `viewport` | viewport | — (all) |
| `full` | scrollback | — (all) |

**Profile keys:**

| Key | Type | Default | Description |
|---|---|---|---|
| `source` | string | `"scrollback"` | `"scrollback"`, `"viewport"`, or `"tab"`. |
| `lines` | integer | *(unbounded)* | Maximum lines to scan. `0` or absent = unbounded. |
| `disable` | string… | *(none)* | Pattern type tags or custom pattern names to skip for this profile. Merged with the global `patterns { disable … }` list. |
| `progress` | bool | `false` | Run one pattern per timer tick (~50 ms each) and show a `LineGauge` progress bar. Matches populate incrementally. Off by default — fast profiles (`quick`, `viewport`) finish before a bar would be visible. Enable for `deep`, `full`, or any profile where extraction takes more than ~300 ms. |

> **KDL syntax note:** each profile property must be on its own line.
> `quick { source "scrollback" lines 150 }` on a single line silently
> drops the `lines` limit because KDL treats `lines` as a third argument
> to `source` rather than a separate key. Use newlines or `;` to separate
> sibling nodes.

**Example:**
```kdl
grab {
    default_profile "quick"
    profiles {
        quick {
            source "scrollback"
            lines 150
        }
        deep {
            source "scrollback"
            lines 1500
            disable "secret" "ipv6"
            progress true
        }
        viewport {
            source "viewport"
        }
        full {
            source "scrollback"
            disable "secret"
            progress true
        }
        tab-scan {
            source "tab"
            lines 150
            disable "secret" "ipv6"
        }
    }
}
```

---

## `limits { }` block

Per-verb caps on multi-target dispatch. Prevents accidental bulk actions
(opening 50 browser tabs, pasting 20 commands at once). Set to `0` to
disable a verb entirely.

| Key | Default | Description |
|---|---|---|
| `copy` | `100` | Max matches for copy-raw / copy-display batch. |
| `insert` | `5` | Max matches for insert-raw / insert-display batch. |
| `open` | `10` | Max matches for open batch. |
| `edit` | `5` | Max matches for edit batch. |
| `reveal` | `10` | Max matches for reveal batch. |
| `json` | `100` | Max matches for JSON export batch. |

**Example:**
```kdl
limits {
    copy   200
    insert   0    // disable insert entirely
    open    10
}
```

---

## `types { }` block

Override verb allow-lists and default `Enter` action per match type.
Keys are type tags (`url`, `file`, `diag`, `sha`, `ipv4`, `ipv6`,
`uuid`, `quote`, `cmd`, `secret`, or any custom pattern name).

### Per-type keys

| Key | Type | Description |
|---|---|---|
| `actions` | string list | Verbs available for this type. Replaces the built-in list. Unknown verb names are silently dropped. |
| `default` | string | Verb fired by `Enter`. If not in the allow-list, falls back to the built-in default. |

**Verb names:** `copy`, `copy-raw`, `copy-display`, `insert`, `insert-raw`,
`insert-display`, `open`, `edit`, `reveal`, `preview`, `json`

**Hardcoded overrides that config cannot change:**
- `copy-raw` and `json` are always allowed for every type.
- `open`, `edit`, `reveal` are always denied for `secret`, regardless of config.

**Example:**
```kdl
types {
    url    { actions "open" "copy" "insert"; default "open" }
    file   { actions "edit" "copy" }
    secret { actions "copy" }    // disable insert for secrets
}
```

---

## `actions { }` block

Command templates for `open`, `edit`, and `reveal` per type. Each
template is passed to `sh -c` (open, reveal) or inserted into the
source pane (edit) so the user can review before hitting Enter.

Keys are type tags; `default` is a fallback for any type not explicitly listed.

### Template variables

| Variable | Resolves to |
|---|---|
| `{editor}` | `$EDITOR` env var, or `"nvim"` if unset |
| `{file}` | File path (from `{file}` field, or raw match) |
| `{line}` | Line number, or empty string if absent |
| `{url}` | URL field |
| `{match}` | Group 1 if regex has groups, otherwise full match |
| `{raw}` | Raw match text (same as `{match}` for built-ins) |
| `{display}` | Display value |
| `{type}` | Type tag string |
| `{context}` | The full line the match appeared on |
| `{0}` | Full regex match (custom patterns) |
| `{1}`, `{2}`, … | Capture groups (custom patterns) |

### `{line}` separator stripping

If `{line}` resolves to empty and is preceded by a separator character
(`:`, `+`, ` `), the separator is stripped automatically:

```
"hx {file}:{line}"   + line=""  → "hx src/main.rs"    (: stripped)
"nvim +{line} {file}" + line=""  → "nvim src/main.rs"  (+ and space stripped)
```

**Example:**
```kdl
actions {
    file    { edit command "hx {file}:{line}" }
    diag    { edit command "hx {file}:{line}" }
    default { edit command "{editor} +{line} {file}" }

    url {
        open command "firefox --new-tab {url}"
    }
}
```

---

## `patterns { }` block

The `patterns` block has three roles: globally disabling patterns, configuring
built-in pattern behaviour, and defining user-defined regex patterns.

### Global `disable`

```kdl
patterns {
    disable "secret" "ipv6"
}
```

`disable` accepts one or more type tags (built-in) or custom pattern names.
Listed patterns are skipped for **every** grab profile. Use per-profile
`disable` (in `grab { profiles { … } }`) to suppress patterns only for
expensive profiles like `deep` or `full`.

**Built-in type tags:** `url`, `file`, `diag`, `sha`, `ipv4`, `ipv6`,
`uuid`, `quote`, `cmd`, `secret`

Custom pattern names match the node name you gave them in `patterns { }`.

### Built-in pattern tuning

#### `command { }` sub-block

Controls the `cmd` type detection.

| Key | Type | Default | Description |
|---|---|---|---|
| `flag_anchored` | bool | `false` | Enable flag-anchored command detection (see below) |

**`flag_anchored` — what it does**

The command pattern has three detection strategies, applied in order:

1. **Prompt-anchored** — line starts with a known prompt marker (`❯ `, `$ `, `> `, `% `, `# `). Always on.
2. **Exec-anchored** — line contains a known trigger word (`git`, `curl`, `docker`, `zellij`, `tmux`, …). Always on.
3. **Flag-anchored** — line contains a `-x`/`-xyz`/`--long-flag` style argument; the command word is found by walking back to the nearest boundary character (`][}{><:;|&`). **Off by default.**

Enable `flag_anchored` when you see commands in your scrollback that aren't caught by the first two strategies — for example, dry-run output, CI log lines, or build system output:

```
[dry-run] rsync -avz src/ dest/     ← no prompt, rsync not in trigger list
Running: ssh -i key user@host       ← no prompt, but ssh IS in trigger list (no change needed)
```

**Why it is off by default:** flag-anchored can produce false positives on prose that incidentally contains flag-looking tokens, e.g. `"Use --verbose for more output"` would match `"Use --verbose for more output"` with command word `Use` — except the uppercase guard rejects it. Most English-prose false positives are caught by requiring the first word to start with a lowercase letter, but edge cases exist. See the [pattern detection notes](../README.md#pattern-detection-and-false-positives) in the README.

```kdl
patterns {
    command {
        flag_anchored true
    }
}
```

### User-defined regex patterns

Each pattern gets its own display label
in the picker and is filterable with `#name`.

### Per-pattern keys

| Key | Required | Description |
|---|---|---|
| `regex` | yes | Regular expression (regex-lite syntax — no lookaround). Invalid patterns are silently skipped. |
| `type` | no | Type tag to assign. Determines available verbs. Default: `"url"`. |
| `template` | no | Template string applied to the match. `{match}` = group 1 or full match. `{0}`, `{1}`, `{2}`, … = capture groups. |

### Capture group semantics

- **No groups** — `{match}` = full regex match
- **One or more groups** — `{match}` = group 1; `{1}` = group 1, `{2}` = group 2, etc.; `{0}` = full match

When a template is present, `raw` (the value used for copy and dedup)
is set to the expanded template result, not the regex match text.

**Example:**
```kdl
patterns {
    // Simple — no groups, no template
    port {
        regex ":[0-9]{4,5}\\b"
        type  "url"
    }

    // Single group (context prefix pattern)
    jira-ticket {
        regex    "New Jira ticket : ([A-Z]+-[0-9]+[A-Z]*)"
        type     "url"
        template "https://jira.example.com/browse/{match}"
    }

    // Multi-group decomposition
    jira {
        regex    "([A-Z]+)-([0-9]+)"
        type     "url"
        template "https://jira.example.com/browse/{1}-{2}"
    }

    github-pr {
        regex    "github\\.com/([^/\\s]+)/([^/\\s]+)/pull/([0-9]+)"
        type     "url"
        template "https://github.com/{1}/{2}/pull/{3}"
    }
}
```

---

## Per-keybind overrides

Settings passed in the Zellij `configuration` map override the config
file for that specific keybind launch.

| Key | Values | Description |
|---|---|---|
| `type` | space-separated type tags | Pre-fill query with `#tag` filters. |
| `preview` | `"on"`, `"off"`, `"always"`, `"never"` | Force preview open or closed, ignoring `ui.preview`. |
| `grab` | profile name string | Start on a specific grab profile, ignoring `grab.default_profile`. |
| `patterns` | space-separated type tags / custom names | **Allowlist mode** — only these patterns run. Overrides all `disable` settings (global and per-profile). Use when a keybind should extract a narrow set of types (e.g. `"url ipv4"` for a URL-only picker). |
| `popupTitle` | string | Override the floating pane title. Default: `"zextract"`. Note: Zellij's own `name` and `title` keys are consumed before they reach the plugin — use `popupTitle` instead. |

**Example:**
```kdl
bind "Alt u" {
    LaunchOrFocusPlugin "file://$HOME/.config/zellij/plugins/zextract.wasm" {
        floating   true;
        patterns   "url ipv4";   // only URL + IPv4 — everything else skipped
        preview    "on";
        popupTitle "URL picker";
    };
}
bind "Alt j" {
    LaunchOrFocusPlugin "file://$HOME/.config/zellij/plugins/zextract.wasm" {
        floating   true;
        type       "jira";       // pre-fill filter (patterns not set → normal extraction)
        grab       "deep";
        popupTitle "JIRA";
    };
}
bind "F" {
    LaunchOrFocusPlugin "file://$HOME/.config/zellij/plugins/zextract.wasm" {
        floating   true;
        grab       "tab-scan";
        patterns   "url file cmd";   // skip secret/sha/uuid on tab-wide grabs
        popupTitle "tab scan";
        move_to_focused_tab true;
    };
}
```

> **`type` vs `patterns`:** `type` pre-fills the query filter (user can still backspace and see other types). `patterns` controls which patterns *run at extraction time* — types not listed produce zero matches regardless of what the query says.
