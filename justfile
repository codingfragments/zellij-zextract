plugin_dir := env_var('HOME') + "/.config/zellij/plugins"

default:
    @just --list

# Build all workspace members for wasm32-wasip1 (release)
build:
    cargo build --release --target wasm32-wasip1

# Build only spike A (write_chars)
build-a:
    cargo build --release --target wasm32-wasip1 -p spike-a-write-chars

# Build only spike B (ratatui)
build-b:
    cargo build --release --target wasm32-wasip1 -p spike-b-ratatui

# Copy both spike wasms into ~/.config/zellij/plugins/
install: build
    mkdir -p {{plugin_dir}}
    rm -f {{plugin_dir}}/spike-a.wasm {{plugin_dir}}/spike-b.wasm
    cp -f $(pwd)/target/wasm32-wasip1/release/spike_a_write_chars.wasm {{plugin_dir}}/spike-a.wasm
    cp -f $(pwd)/target/wasm32-wasip1/release/spike_b_ratatui.wasm {{plugin_dir}}/spike-b.wasm
    @echo "Installed:"
    @ls -la {{plugin_dir}}/spike-a.wasm {{plugin_dir}}/spike-b.wasm

# Build + reload spike A in a running zellij
dev-a: build-a
    ln -sf $(pwd)/target/wasm32-wasip1/release/spike_a_write_chars.wasm {{plugin_dir}}/spike-a.wasm
    zellij action reload-plugin spike-a 2>/dev/null || echo "Plugin not loaded yet; bind it first"

# Build + reload spike B in a running zellij
dev-b: build-b
    ln -sf $(pwd)/target/wasm32-wasip1/release/spike_b_ratatui.wasm {{plugin_dir}}/spike-b.wasm
    zellij action reload-plugin spike-b 2>/dev/null || echo "Plugin not loaded yet; bind it first"

clean:
    cargo clean
