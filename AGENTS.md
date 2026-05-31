# Flovenet — Agent Instructions

## Build & Verify Commands

```bash
# CI pipeline order (all must pass):
cargo fmt --check
cargo clippy --all-targets -- -D warnings    # zero warnings enforced
cargo build --all-targets
cargo test                                    # ~178 tests

# Additional CI checks:
cargo audit
cargo deny check
```

Always run `fmt`, then `clippy`, then `test` before considering a change done. CI rejects any clippy warning.

## Workspace Layout

- **18 workspace members** in root `Cargo.toml` — all standard Rust crates
- **`wasm_images/`** is **excluded** from the workspace. Build WASM modules separately:
  ```bash
  cd wasm_images/feed_ranker && cargo build --target wasm32-wasi --release
  ```
- **`web-dashboard/`** is a separate Node.js project (React 19 + Vite 6 + urql):
  ```bash
  cd web-dashboard && npm install && npm run build   # runs tsc && vite build
  ```
  Dev server (`npm run dev`) proxies `/graphql` to `http://localhost:8080`.

## Binary Entrypoints

The `daemon` crate compiles to the main binary. Docker renames it to `flovenet`. All CLI subcommands (`daemon`, `api-gateway`, `share`, `run`, `status`) are dispatched from this single binary via clap.

```bash
cargo run --release -- daemon --port 0 --api-port 9090 --roles compute,storage
cargo run --release -- api-gateway --port 8080
```

## Key Crate Dependencies

- `daemon` depends on nearly every other crate — changes to any crate likely trigger a daemon rebuild.
- `graphql_api` depends on `social_protocol`, `identity`, `crypto`, `storage`, `resource_manager`.
- `flovenet-core` is the only crate with JNI (`cfg(target_os = "android")`) — produces both `lib` and `cdylib`.

## Conventions

- `use` order: std → external → internal
- `async_trait` for async traits
- `thiserror` for crate-level errors, `anyhow` at boundaries / in binaries
- Serde derive on all data structs
- Unit tests inline (`#[cfg(test)] mod tests`), integration tests in crate `tests/` dirs

## Integration Tests

`tests/docker_integration_test.py` — Python script that requires `docker compose up` running (3 nodes + gateway). Uses `sudo docker exec`. Not part of `cargo test`.

## Android

`scripts/build-android.sh` requires Android NDK 27+ at `/home/x/android-sdk/ndk/27.0.12077973`. Cross-compiles `flovenet-core` for `aarch64-linux-android`, copies `.so` to `android/app/src/main/jniLibs/arm64-v8a/`, then runs Gradle.
