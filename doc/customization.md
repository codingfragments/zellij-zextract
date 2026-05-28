# Customization guide

How to tailor zextract with `actions { }`, `types { }`, `patterns { }`,
and `grab { }` blocks. See [config-reference.md](config-reference.md)
for the complete key listing.

---

## `actions { }` — Custom editor and open commands

The `actions` block overrides what happens when you press `e` (edit),
`o` (open), or `r` (reveal) on a match.

### Choosing an editor

The built-in default is `{editor} +{line} {file}` where `{editor}`
resolves to `$EDITOR` or `nvim`. To use a different editor for all
file-like types:

```kdl
actions {
    default { edit command "hx {file}:{line}" }
}
```

Per-type overrides win over `default`:

```kdl
actions {
    diag    { edit command "hx {file}:{line}" }     // helix for diagnostics
    default { edit command "{editor} +{line} {file}" }  // $EDITOR for everything else
}
```

### VSCode

```kdl
actions {
    default { edit command "code -g {file}:{line}" }
}
```

### JetBrains (idea / goland / etc.)

```kdl
actions {
    default { edit command "idea --line {line} {file}" }
}
```

### Custom browser for URLs

```kdl
actions {
    url { open command "firefox --new-tab {url}" }
}
```

### `{line}` separator stripping

If `{line}` is absent (a plain file path with no `:42` suffix), any
separator character immediately before `{line}` in the template is
automatically stripped:

| Template | No line | With line=42 |
|---|---|---|
| `"hx {file}:{line}"` | `hx src/main.rs` | `hx src/main.rs:42` |
| `"nvim +{line} {file}"` | `nvim src/main.rs` | `nvim +42 src/main.rs` |
| `"code -g {file}:{line}"` | `code -g src/main.rs` | `code -g src/main.rs:42` |

Separator chars that are stripped: `:`, `+`, ` ` (space), `,`.

---

## `types { }` — Per-type verb allow-lists

Control which actions appear in the footer for each type and which fires
on `Enter`.

### Restrict URL matches to open+copy only

```kdl
types {
    url { actions "open" "copy"; default "open" }
}
```

### Disable insert for secrets

```kdl
types {
    secret { actions "copy" }
}
```

Note: `open`, `edit`, and `reveal` are always denied for `secret`
regardless of this config — that is a hardcoded safety rule.

### Make copy the default for SHAs

```kdl
types {
    sha { actions "copy" "insert"; default "copy" }
}
```

### Block a verb entirely with `limits`

Set the verb's limit to `0` to refuse it even for single matches:

```kdl
limits { insert 0 }   // disable insert for all types
```

---

## `patterns { }` — Custom regex patterns

Define new pattern types beyond the ten built-ins.

### Simple pattern (no groups, no template)

Matches port numbers like `:3000`, `:8080`:

```kdl
patterns {
    port {
        regex ":[0-9]{4,5}\\b"
        type  "url"
    }
}
```

- **`{match}`** = the matched text (`:3000`)
- No template → `raw` = `{match}`
- Appears as `[port]  :3000` in the picker
- Filterable with `#port`

### Single capture group (context-anchored)

Match only when a known prefix is present, but capture just the value:

```kdl
patterns {
    jira-new {
        regex    "New Jira ticket : ([A-Z]+-[0-9]+[A-Z]*)"
        type     "url"
        template "https://jira.example.com/browse/{match}"
    }
}
```

The full line `New Jira ticket : ST-154R` triggers the match, but
`{match}` = `ST-154R` (group 1). Template expands to the Jira URL.
`raw` = the expanded URL (used for copy and dedup).

### Multiple capture groups

Decompose a match into named parts:

```kdl
patterns {
    github-pr {
        regex    "github\\.com/([^/\\s]+)/([^/\\s]+)/pull/([0-9]+)"
        type     "url"
        template "https://github.com/{1}/{2}/pull/{3}"
    }
}
```

For `github.com/myorg/myrepo/pull/99`:
- `{0}` = `github.com/myorg/myrepo/pull/99` (full match)
- `{1}` = `myorg`
- `{2}` = `myrepo`
- `{3}` = `99`
- `{match}` = `myorg` (group 1, also `raw` before template)
- template result = `https://github.com/myorg/myrepo/pull/99`
- `raw` = `https://github.com/myorg/myrepo/pull/99` (expanded, used for dedup)

Each unique PR URL is a distinct entry in the picker, even if the
same org appears in multiple PR references.

### Pattern with file type

Open a matched path in the editor:

```kdl
patterns {
    makefile-target {
        regex "Makefile:([0-9]+)"
        type  "diag"
    }
}
```

This gives the match `edit` as its default action (diag type), and
`{line}` is populated from group 1.

### Filtering custom patterns

Custom pattern names are added to the `#filter` tag set automatically.
Typing `#port` or `#github-pr` filters to just those matches. Prefix
matching works too: `#ji` resolves to `#jira` if it's unambiguous.

---

## Disabling patterns

### Global disable

Skip a pattern for every profile and every keybind that doesn't override it:

```kdl
patterns {
    disable "ipv6" "uuid"
}
```

Built-in type tags: `url`, `file`, `diag`, `sha`, `ipv4`, `ipv6`,
`uuid`, `quote`, `cmd`, `secret`. Custom pattern names work too.

### Per-profile disable

Disable expensive patterns only on large grabs:

```kdl
grab {
    profiles {
        quick {
            source "scrollback"
            lines 150
        }
        deep {
            source "scrollback"
            lines 1500
            disable "secret" "ipv6"   // skip on big grabs
        }
        full {
            source "scrollback"
            disable "secret" "ipv6"
        }
    }
}
```

Per-profile `disable` is merged with the global list — you can't
re-enable a globally disabled pattern from a profile.

### Per-keybind allowlist (`patterns`)

For a dedicated picker keybind, use `patterns` instead of `disable`.
It is an **allowlist**: only the listed patterns run, ignoring all
`disable` settings entirely.

```kdl
bind "u" {
    LaunchOrFocusPlugin "file:~/.config/zellij/plugins/zextract.wasm" {
        floating   true;
        patterns   "url ipv4";   // only these two patterns run
        preview    "on";
        popupTitle "URL picker";
    };
    SwitchToMode "locked"
}
```

> **`type` vs `patterns`:**
> `type "url"` pre-fills the query with `#url` — the user can backspace
> it away and see everything that was extracted. `patterns "url ipv4"`
> controls *what gets extracted*: patterns not listed produce zero
> matches regardless of the query.
>
> You can combine both: `patterns "url ipv4"` for extraction scope and
> `type "url"` to open with the list already filtered to URLs.

---

## Combining everything

A full config for a team using JIRA, GitHub, and helix:

```kdl
log_level "info"

actions {
    diag    { edit command "hx {file}:{line}" }
    file    { edit command "hx {file}:{line}" }
    default { edit command "{editor} +{line} {file}" }
    url     { open command "open {url}" }
}

types {
    url    { actions "open" "copy" "insert"; default "open" }
    file   { actions "edit" "copy" }
    secret { actions "copy" }
}

limits { insert 5; open 10 }

patterns {
    jira {
        regex    "([A-Z]+)-([0-9]+)"
        type     "url"
        template "https://your-company.atlassian.net/browse/{1}-{2}"
    }
    github-pr {
        regex    "github\\.com/([^/\\s]+)/([^/\\s]+)/pull/([0-9]+)"
        type     "url"
        template "https://github.com/{1}/{2}/pull/{3}"
    }
}

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
        }
        viewport {
            source "viewport"
        }
        full {
            source "scrollback"
            disable "secret" "ipv6"
        }
    }
}
```
