# Built-in match types

zextract scans scrollback for ten built-in types. Each type has a short
tag used in `#filters`, `types { }` blocks, and `actions { }` templates.

---

## `url` — URLs and URIs

**Tag:** `url`

**Matches:** `http://`, `https://`, `ftp://`, `file://`, `git://`, `ssh://` URIs.

**Examples:**
```
https://github.com/codingfragments/zellij-zextract
http://localhost:3000/api/v1
git://git.kernel.org/pub/scm/linux/kernel/git/torvalds/linux.git
```

**Default action:** `open` (browser / `open` command)

**Available verbs:** open, copy, insert

**Template fields:**
| Field | Value |
|---|---|
| `{url}` | Full URL |
| `{scheme}` | e.g. `https` |
| `{host}` | e.g. `github.com` |
| `{match}` / `{raw}` | Same as `{url}` |

---

## `file` — File paths

**Tag:** `file`

**Matches:** Paths with at least one `/`. Bare filenames without a slash
(`Cargo.toml`, `stefan.marx`) are intentionally excluded to reduce noise.
Optional `:line` or `:line:col` suffix.

**Examples:**
```
/etc/passwd
~/dotfiles/zextract.kdl
./src/main.rs
src/main.rs:42:8
../Cargo.toml
```

**Does not match:** `Cargo.toml`, `call.json()`, `v1.2`
Add `./` prefix to force-match a bare filename.

**Default action:** `edit`

**Available verbs:** edit, copy, insert

**Template fields:**
| Field | Value |
|---|---|
| `{file}` | Path without line/col suffix |
| `{line}` | Line number, or empty |
| `{col}` | Column number, or empty |
| `{dir}` | Parent directory |
| `{basename}` | Filename with extension |
| `{ext}` | Extension without dot |

---

## `diag` — Compiler / linter diagnostics

**Tag:** `diag`

**Matches:** `file:line:col` triples from compiler output. Requires all
three parts. Distinguishes from plain file paths by presence of column.

**Examples:**
```
src/main.rs:42:8
crates/zextract/src/pattern/url.rs:16:5
```

**Default action:** `edit` (jumps directly to the line in `$EDITOR`)

**Available verbs:** edit, copy, insert

**Template fields:** same as `file` — `{file}`, `{line}`, `{col}`, `{dir}`, `{basename}`, `{ext}`

---

## `sha` — Git commit hashes

**Tag:** `sha`

**Matches:** Hexadecimal strings of 7–64 chars that look like git SHAs.
Pure-numeric strings are excluded.

**Examples:**
```
a1b2c3d
e4f5g6h7i8j9k0l1m2n3o4p5q6r7s8t9u0v1w2x3
```

**Default action:** insert

**Available verbs:** copy, insert

**Template fields:** `{match}` / `{raw}` — the hash

---

## `ipv4` — IPv4 addresses

**Tag:** `ipv4`

**Matches:** Dotted-quad notation with optional `:port`. Octets must be
in range 0–255. Port must be ≤ 65535.

**Examples:**
```
192.168.1.1
10.0.0.1:8080
127.0.0.1
```

**Default action:** insert

**Available verbs:** copy, insert

**Template fields:** `{match}` — the full address including port if present

---

## `ipv6` — IPv6 addresses

**Tag:** `ipv6`

**Matches:** Full and compressed IPv6 notation, optionally bracketed with
port (`[::1]:8080`).

**Examples:**
```
::1
2001:db8::1
[::1]:8080
fe80::1%eth0
```

**Default action:** insert

**Available verbs:** copy, insert

**Template fields:** `{match}` — the address

---

## `uuid` — UUIDs

**Tag:** `uuid`

**Matches:** Standard hyphenated UUID format (case-insensitive).

**Examples:**
```
550e8400-e29b-41d4-a716-446655440000
6ba7b810-9dad-11d1-80b4-00c04fd430c8
```

**Default action:** insert

**Available verbs:** copy, insert

**Template fields:** `{match}` — the UUID

---

## `quote` — Quoted strings

**Tag:** `quote`

**Matches:** Text enclosed in `"..."`, `'...'`, or `` `...` ``. The only
type where `raw` and `display` differ: `raw` includes the surrounding
quotes; `display` is the unquoted content.

**Examples:**
```
"hello world"
'single quoted'
`backtick`
```

**Default action:** insert

**Available verbs:** copy-raw (with quotes), copy-display (without quotes),
insert-raw, insert-display

**Template fields:** `{match}` / `{raw}` — with quotes; `{display}` — without quotes

---

## `cmd` — Shell commands

**Tag:** `cmd`

**Matches:** Two patterns:

1. **Prompt-anchored** — text following a prompt marker (`❯`, `$`, `>`,
   `%`, `#`). Most reliable.
2. **Executable-anchored** — lines starting with a curated trigger list
   (`sudo`, `curl`, `wget`, `cat`, `git`, `kubectl`, `cargo`, `make`, …).

Multi-line commands joined by `\` continuation are spliced into one match
(up to 10 lines), with line-number prefixes, diff markers, and comment
prefixes stripped from continuation lines.

**Examples:**
```
❯ git push origin main
$ cargo build --release --target wasm32-wasip1
kubectl get pods -n production
```

**Default action:** insert (pastes to source pane prompt for review)

**Available verbs:** insert, copy

**Template fields:** `{match}` — the full command text

---

## `secret` — Credentials and tokens

**Tag:** `secret`

**Matches:** Two strategies:

1. **Known formats** — JWT, AWS access/secret keys, GitHub tokens (`ghp_`,
   `gho_`, `ghs_`), GitLab PATs, Stripe keys, OpenAI keys, Anthropic keys,
   Slack tokens, Bearer tokens.
2. **Entropy fallback** — strings of 20–200 chars with ≥3 character classes
   and Shannon entropy ≥ 3.5 bits/char. Catches unknown formats.

**Security note:** `open`, `edit`, and `reveal` are **hardcoded denied**
for secrets — this cannot be overridden via config, even with
`types { secret { actions "open" } }`.

**Default action:** copy (no insert by default — avoid leaking to terminal history)

**Available verbs:** copy, insert *(open/edit/reveal always denied)*

**Template fields:** `{match}` — the token; `{secret_format}` — detected format if known

---

## Cross-type dedup

When two pattern types match the same text, the higher-priority type wins.
Priority order (highest first):

```
url > diag > file > uuid > sha > ipv4 > ipv6 > cmd > secret > quote
```

This means a string like `https://example.com` is classified as `url`
even if it also matches the `quote` pattern when surrounded by quotes.

---

## Type color reference

| Type | Color |
|---|---|
| `url` | Blue |
| `file` | Green |
| `diag` | Light red |
| `sha` | Yellow |
| `ipv4` | Cyan |
| `ipv6` | Cyan |
| `uuid` | Magenta |
| `quote` | Gray |
| `cmd` | Light magenta |
| `secret` | Light red |
| Custom patterns | Inherited from their `type` |
