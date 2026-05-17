# Use cases

Worked walkthroughs showing zextract in real scenarios.

---

## Open a URL from build output

You're watching `cargo run` output and spot a localhost URL in the logs:

```
     Running server on http://localhost:3000
     API available at http://127.0.0.1:8080/api/v1
```

**Flow:**
1. Press `Alt-x` to open the picker.
2. The two localhost URLs appear at the top (most recent first).
3. Navigate to `http://localhost:3000`, press `Enter` → opens in browser.

**Tip:** Wire a dedicated URL-only keybind so the picker opens
pre-filtered:

```kdl
bind "Alt u" {
    LaunchOrFocusPlugin "file://$HOME/.config/zellij/plugins/zextract.wasm" {
        floating true; type "url"; preview "on";
    };
}
```

---

## Jump to a diagnostic in the editor

A compiler error points to a file and line:

```
error[E0382]: borrow of moved value
  --> src/main.rs:142:8
```

**Flow:**
1. Open picker (`Alt-x`).
2. Type `#diag` to show only diagnostics.
3. Navigate to `src/main.rs:142:8`, press `Enter` → inserts
   `hx src/main.rs:142` (or your configured editor) into the prompt.
4. Hit Enter in the source pane to open the file at the right line.

**Config for helix:**
```kdl
actions {
    diag { edit command "hx {file}:{line}" }
}
```

**Config for VSCode:**
```kdl
actions {
    diag { edit command "code -g {file}:{line}" }
}
```

---

## Insert a command back to the prompt for review

You see a long `kubectl` command in your scrollback and want to re-run
it with modifications:

```
kubectl get pods -n production --field-selector=status.phase=Running
```

**Flow:**
1. Open picker.
2. Type `#cmd` to filter to commands.
3. Navigate to the kubectl line, press `i` (insert-raw).
4. The command lands at your prompt — edit as needed, then Enter.

---

## Copy multiple file paths as JSON for scripting

You have a build failure listing several affected files and want to
pipe them to a script:

```
error in src/auth/login.rs:42
error in src/auth/session.rs:17
error in src/api/routes.rs:88
```

**Flow:**
1. Open picker, type `#file`.
2. Press `Space` on each file match to multi-select.
3. Press `J` — all selected matches exported as a JSON array to clipboard.
4. Paste into your script or `jq` pipeline.

**JSON output shape:**
```json
[
  {"type":"file","raw":"src/auth/login.rs:42","file":"src/auth/login.rs","line":"42"},
  {"type":"file","raw":"src/auth/session.rs:17","file":"src/auth/session.rs","line":"17"},
  {"type":"file","raw":"src/api/routes.rs:88","file":"src/api/routes.rs","line":"88"}
]
```

---

## Wire a dedicated JIRA keybind

Your team uses JIRA ticket references everywhere in commit messages and
PR descriptions. You want `Alt-j` to instantly show all ticket refs in
the scrollback, expanded to clickable URLs.

**Step 1** — Add the pattern to `~/.config/zellij/zextract.kdl`:
```kdl
patterns {
    jira {
        regex    "([A-Z]+)-([0-9]+)"
        type     "url"
        template "https://your-company.atlassian.net/browse/{1}-{2}"
    }
}
```

**Step 2** — Add the keybind to `~/.config/zellij/config.kdl`:
```kdl
bind "Alt j" {
    LaunchOrFocusPlugin "file://$HOME/.config/zellij/plugins/zextract.wasm" {
        floating true;
        type "jira";
        grab "deep";    // search 1500 lines back
    };
}
```

**Result:** `Alt-j` opens a picker showing only JIRA matches like
`[jira]  https://your-company.atlassian.net/browse/PROJ-123`.
Press `Enter` → opens in browser.

---

## Extract a context-anchored value

Your logs prefix ticket refs with a label, and you only want the ID
not the whole prefix:

```
New Jira ticket : ST-154R    assigned to alice
Blocked by      : BACKEND-42 waiting on API contract
```

Use a **context-prefix pattern** with a capture group:

```kdl
patterns {
    jira-new {
        regex    "New Jira ticket : ([A-Z]+-[0-9]+[A-Z]*)"
        type     "url"
        template "https://your-company.atlassian.net/browse/{match}"
    }
}
```

The prefix `New Jira ticket : ` is required to trigger the match, but
only the captured group (`ST-154R`) becomes the match value. The picker
shows `[jira-new]  https://...atlassian.net/browse/ST-154R`.

---

## Widen search when there are no matches

The picker shows "No matches in pane scrollback" — your terminal only
keeps a short viewport buffer.

**Options in order:**
1. Press `g` (List mode) or `Alt-g` (either mode) to cycle to the
   `deep` profile (1500 lines).
2. Press again for `viewport` (current terminal viewport only).
3. Press again for `full` (entire scrollback, no cap).

The grab label `[quick]` outside the input box updates with each cycle.
The picker re-extracts immediately.
