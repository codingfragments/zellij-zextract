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
| `source` | string | `"scrollback"` | `"scrollback"` or `"viewport"`. |
| `lines` | integer | *(unbounded)* | Maximum lines to scan. `0` or absent = unbounded. |

**Example:**
```kdl
grab {
    default_profile "quick"
    profiles {
        quick    { source "scrollback"  lines 150  }
        deep     { source "scrollback"  lines 1500 }
        viewport { source "viewport"               }
        full     { source "scrollback"             }
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

User-defined regex patterns. Each pattern gets its own display label
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
| `popupTitle` | string | Override the floating pane title. Default: `"zextract"`. Note: Zellij's own `name` and `title` keys are consumed before they reach the plugin — use `popupTitle` instead. |

**Example:**
```kdl
bind "Alt u" {
    LaunchOrFocusPlugin "file://$HOME/.config/zellij/plugins/zextract.wasm" {
        floating   true;
        type       "url";
        preview    "on";
        popupTitle "URL picker";
    };
}
bind "Alt j" {
    LaunchOrFocusPlugin "file://$HOME/.config/zellij/plugins/zextract.wasm" {
        floating   true;
        type       "jira";
        grab       "deep";
        popupTitle "JIRA";
    };
}
```
