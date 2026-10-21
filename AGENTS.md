# Working in this repository

Azeroth Survivors is a top-down survivors-like roguelite set in WoW 1.12.1. It runs on
[benilla](https://github.com/samwhosung/benilla), a from-scratch WoW 1.12.1 client in Rust + Bevy,
by way of [wenilla](https://github.com/Arnesen/wenilla), the fork that adds the browser build and
a local host. The game mode is our layer on top: no server, no login. It reads art, models and
sounds from a user-supplied WoW 1.12.1 client at runtime. The design follows the fan game by
[Aethus](https://wow.aethus.pl/); this is a separate implementation, not Aethus's code.

This repo is `Jawfish/azeroth-survivors-re`, a fork of `OutThisLife/azeroth-survivors-re`. This
file is authoritative for the game mode and the repo's process. Upstream's rules for the engine
crates are `docs/METHOD.md`, `docs/CONTRIBUTING.md` and the generated `docs/MAP.md`; read them
before changing engine code.

## What lives where

| path | what |
|---|---|
| `crates/benilla-app/src/survivors/` | the game mode: most of our work happens here |
| `crates/benilla-app/src/survivors/data.rs` | every gameplay table and number: classes, abilities, passives, enemies, bosses, zones, wave clock, XP curve |
| `crates/warcraft-survivors` | native launcher; calls `benilla_app::run_survivors` |
| `crates/wenilla` | browser launcher (`#[wasm_bindgen(start)]`); calls the same `run_survivors` |
| `crates/wenilla-host` | local dev server: static files, `/data/*` from the MPQs, `/ws/*` relay. Loopback only. |
| `crates/benilla-*` | the engine: formats, world, UI, protocol, app (upstream's, plus wenilla's `#[cfg]`-gated carries) |
| `crates/benilla` | the stock client launcher, not the game |
| `crates/benilla-app/src/webbridge/` | the JavaScript bridge, wasm side |
| `web/` | the page: `index.html`, `boot.js`, `platform.js`, `bridge.js`, `examples/`, `tests/`, `wasi_stubs.js` |
| `scripts/fetch-data.sh` | downloads the enUS 1.12.1 (5875) client and extracts the MPQs into `wow-data/Data/` |
| `scripts/web-setup.sh`, `scripts/web-build.sh` | wasm toolchain and build |
| `third_party/` | patched `lua-src`, `mlua-sys`, `kira`; each keeps its own license |
| `reference/` | 1.12 API surface tables the engine tracks |
| `.github/workflows/check.yml` | wasm check, host and page tests, client-carry tests (PRs and pushes to `main`) |
| `.github/workflows/ci.yml` | upstream's gates: fmt, clippy, tests (PRs from forks) |

## Commands

```bash
just build                  # fetch-data + toolchain + wasm (~5.5 GB download the first time)
just start                  # http://127.0.0.1:8090/ — serves web/dist with wenilla-host
just native                 # native desktop build of warcraft-survivors
scripts/web-build.sh        # → web/dist. WEB_DEBUG=1 keeps symbols for readable stack traces.
cargo check --target wasm32-unknown-unknown -p wenilla --no-default-features --features webgpu
                            # the wasm build without linking; what check.yml runs
cargo check --workspace --all-targets      # native (Debian/Ubuntu: libasound2-dev libudev-dev)
cargo test --workspace                     # ~320 tests need a 1.12.1 patch chain and fail without one
```

Recipes are in `justfile`; `just` lists them. Runtime switches: `WOW_DATA` (the `Data` path, native), `WOW_GRADE=0` / `?grade=0` (picture grade
off), `WOW_SURVIVORS_AUTO=<Class>[:idle|:all|:late]` (a bot plays a run and logs every 10 s; see
`survivors/auto.rs`), `WOW_SURVIVORS_MAP=<index or name>`, `WOW_SURVIVORS_SHOTS=<dir>`. Use the
auto run to check a balance change.

A browser for testing needs WebGPU; the world does not run without it. Linux Chrome needs
`--enable-unsafe-webgpu`. Without an adapter the client aborts in
`wgpu::create_bind_group_layout` at world entry: that is the missing adapter, not your change.
A throwaway profile with a devtools port, for driving it from a script:

```bash
google-chrome --user-data-dir=/tmp/wenilla-chrome --enable-unsafe-webgpu --remote-debugging-port=9333 http://127.0.0.1:8090/
```

## Invariants

- **No Blizzard content in the repo.** No art, models, sounds, maps, MPQ contents, FrameXML or
  GlueXML. The game reads them from the user's client at runtime, and never writes into it.
- **Gameplay numbers are ours, not WoW's.** A spell id in `data.rs` selects only an icon, a name
  and a visual chain from `Spell.dbc`. The other zones derive enemy health, damage and XP from
  The Barrens' tuned ladder (`ENEMIES`), so a change there moves every zone.
- **Both launchers call `run_survivors`.** Behaviour that differs between web and native differs
  inside `benilla-app`, behind `#[cfg(target_arch = "wasm32")]`, never in a launcher.
- **Keep engine changes small and gated.** Prefer the survivors module or a new file. A change
  inside an engine file goes under a `#[cfg]` where it can. That keeps a future merge from
  benilla or wenilla cheap (`docs/UPSTREAM.md` is wenilla's merge process; this repo has no
  `upstream` remote yet).
- **No threads, no std time on wasm.** `std::time::Instant`, `SystemTime` and `env::vars_os`
  panic there; use `bevy::platform::time::Instant` or `web-time`. Read env vars through
  `crate::webenv::var`, which also reads the page's query string.
- **What ships is the `cp` line in `scripts/web-build.sh`.** A page file that is not on it never
  reaches `web/dist`.
- **`wasm-bindgen` crate and CLI versions must match exactly**; `web-setup.sh` enforces it.
- **`ring_reaction` returns 0..7**; every consumer adds `+ 1` for the Lua 1..8 scale.
- **`gh pr create` needs `-R Jawfish/azeroth-survivors-re`.** Without it `gh` targets the fork's
  parent.

## Where to look when

| symptom | look at |
|---|---|
| a gameplay value is wrong or needs tuning | `survivors/data.rs`; the mobility level is `survivors/mobility.rs` |
| the wasm panics at boot with `RuntimeError: unreachable` | rebuild with `WEB_DEBUG=1`; check for a WebGPU adapter first; `?bridge=0` rules the bridge out |
| wasm-bindgen glue errors at load | version drift: rerun `scripts/web-setup.sh` |
| a change builds natively and not for wasm | `cargo check --target wasm32…`; look for std time, threads or a native-only crate |
| no game data / the world does not load | `just fetch-data`; `wow-data/Data/` must hold `terrain.MPQ` and `patch.MPQ` |
| no sound in the browser | the page resumes the AudioContext on the first gesture; the mixer backend is per target in `sound/mixer.rs` |
| the world freezes ~200 ms at zone lines | the soundscape carry (`sound/zone.rs`, `sound/web_load.rs`), wasm-only |
| short stalls on the first play of a sound in the browser | the SFX carry (`sound/kit/deferred.rs`), wasm-only |
