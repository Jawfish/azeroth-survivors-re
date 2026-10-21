set shell := ["bash", "-euo", "pipefail", "-c"]

data_dir := "wow-data"
data := data_dir / "Data"
wasm := "web/dist/wenilla_bg.wasm"

[private]
default:
    @just --list --unsorted

# Download the game data (a WoW 1.12.1 client) from archive.org
fetch-data:
    ./scripts/fetch-data.sh

# Install the wasm toolchain (wasi-sdk, wasm-bindgen, binaryen)
toolchain:
    ./scripts/web-setup.sh

# Build the browser client into web/dist/
wasm:
    ./scripts/web-build.sh

# Fetch data + install toolchain + build the wasm
build: fetch-data toolchain wasm

# Serve the game at http://127.0.0.1:8090 (run `just build` first)
start:
    @test -f {{wasm}} || { echo "  not built yet — run: just build"; exit 1; }
    cargo run --release -p wenilla-host -- --www web/dist --data {{data}}

# Build + run the native desktop version
native: fetch-data
    WOW_DATA={{data}} cargo run --release -p warcraft-survivors

# Check the native workspace
check:
    cargo check --locked --workspace --all-targets

# Check the browser build without linking (what check.yml runs)
check-wasm:
    cargo check --locked --target wasm32-unknown-unknown -p wenilla --no-default-features --features webgpu

# Run strict linting
lint:
    cargo clippy --locked --workspace --all-targets -- -D warnings

# Format code
fmt:
    cargo fmt --all

# Check formatting
fmt-check:
    cargo fmt --all -- --check

# Remove the fetched game data and wasm build output
clean:
    rm -rf web/dist {{data_dir}}
