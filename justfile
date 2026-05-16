plugin_dir := env_var('HOME') + "/.config/zellij/plugins"

default:
    @just --list

# Build all workspace members for wasm32-wasip1 (release)
build:
    cargo build --release --target wasm32-wasip1

# Build only zextract
build-zextract:
    cargo build --release --target wasm32-wasip1 -p zextract

# Build only spike A (write_chars)
build-a:
    cargo build --release --target wasm32-wasip1 -p spike-a-write-chars

# Build only spike B (ratatui)
build-b:
    cargo build --release --target wasm32-wasip1 -p spike-b-ratatui

# Copy zextract + both spike wasms into ~/.config/zellij/plugins/
install: build
    mkdir -p {{plugin_dir}}
    rm -f {{plugin_dir}}/zextract.wasm {{plugin_dir}}/spike-a.wasm {{plugin_dir}}/spike-b.wasm
    cp -f $(pwd)/target/wasm32-wasip1/release/zextract.wasm {{plugin_dir}}/zextract.wasm
    cp -f $(pwd)/target/wasm32-wasip1/release/spike_a_write_chars.wasm {{plugin_dir}}/spike-a.wasm
    cp -f $(pwd)/target/wasm32-wasip1/release/spike_b_ratatui.wasm {{plugin_dir}}/spike-b.wasm
    @echo "Installed:"
    @ls -la {{plugin_dir}}/zextract.wasm {{plugin_dir}}/spike-a.wasm {{plugin_dir}}/spike-b.wasm

# Install zextract only
install-zextract: build-zextract
    mkdir -p {{plugin_dir}}
    rm -f {{plugin_dir}}/zextract.wasm
    cp -f $(pwd)/target/wasm32-wasip1/release/zextract.wasm {{plugin_dir}}/zextract.wasm
    @ls -la {{plugin_dir}}/zextract.wasm

# Build + reload zextract in a running zellij
dev: build-zextract
    rm -f {{plugin_dir}}/zextract.wasm
    cp -f $(pwd)/target/wasm32-wasip1/release/zextract.wasm {{plugin_dir}}/zextract.wasm
    zellij action reload-plugin zextract 2>/dev/null || echo "Plugin not loaded yet; bind it first"

# Build + reload spike A in a running zellij
dev-a: build-a
    rm -f {{plugin_dir}}/spike-a.wasm
    cp -f $(pwd)/target/wasm32-wasip1/release/spike_a_write_chars.wasm {{plugin_dir}}/spike-a.wasm
    zellij action reload-plugin spike-a 2>/dev/null || echo "Plugin not loaded yet; bind it first"

# Build + reload spike B in a running zellij
dev-b: build-b
    rm -f {{plugin_dir}}/spike-b.wasm
    cp -f $(pwd)/target/wasm32-wasip1/release/spike_b_ratatui.wasm {{plugin_dir}}/spike-b.wasm
    zellij action reload-plugin spike-b 2>/dev/null || echo "Plugin not loaded yet; bind it first"

# Run cargo tests (host-native target — extraction logic only)
test:
    cargo test -p zextract

# Run the same checks as CI: fmt, clippy, test, wasm build
check:
    cargo fmt --all -- --check
    cargo clippy --all-targets -- -D warnings
    cargo test
    cargo build --release --target wasm32-wasip1

clean:
    cargo clean

# Remove Zellij's cached compiled-module entries for our plugins.
# Zellij caches by load path, not content hash, so a rebuild does not
# invalidate the cache. Run after any ABI-affecting change (zellij-tile
# bump, crate-type change, ...) to force recompile on next launch.
clear-cache:
    find "$HOME/Library/Caches/org.Zellij-Contributors.Zellij/" \
        \( -path '*spike-a*' -o -path '*spike-b*' -o -path '*zextract*' \) \
        -exec rm -rf {} + 2>/dev/null || true
    @echo "Cleared zellij compiled-module cache for spike-a, spike-b, zextract."
