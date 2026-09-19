# Spike notes — MaaCore FFI + PlayTools

Date: 2026-09-19. Runner: `cargo run -p arkd-core --example spike -- <step>` in `crates/arkd-core`.
Library: `/Applications/MAA.app/Contents/Frameworks/libMaaCore.dylib` (loaded by absolute path, no DYLD vars needed).
Resource: `/Applications/MAA.app/Contents/Resources` (parent of `resource/`). User dir: `state/spike/`.

## Results

| Step | Result | Numbers |
|---|---|---|
| load | PASS | `Assistant::load` + `get_version` → `v6.17.5`, 194 ms total |
| resource | PASS | `set_user_dir` + `load_resource`, 479 ms |
| callback | PASS | instance created and dropped, no crash; 1 callback received during create/drop |
| two-instances | PASS | two `Assistant`s alive simultaneously, both dropped cleanly |
| connect | SKIP | `127.0.0.1:1717` not listening — game/MaaTools not running at spike time |
| playtools-concurrent | SKIP | same reason |
| hud-sample | SKIP | needs formation screen + game running |

Connect-path numbers are pending a live game; rerun `cargo run -p arkd-core --example spike -- connect` with MaaTools on `:1717`.

## API facts confirmed against maa-cli @ aebb5e9

- `maa_core::Assistant::load(path)` / `loaded()` / `unload()` wrap `maa_sys::binding::load` with
  an active-instance counter; unload fails while instances are alive.
- `Callback` trait: `fn on_message(&self, kind: MessageKind, msg: Option<&str>)`, `Send + Sync`.
  Blanket impls for `Fn(...)` closures and `Arc<C: Callback>`. Panic inside callback aborts the process.
- `InstanceOptionKey` variants: `TouchMode=2, DeploymentWithPause=3, AdbLiteEnabled=4, KillAdbOnExit=5, ClientType=6`.
- `StaticOptionKey`: `CpuOCR=1, GpuOCR=2`.
- `TouchMode::MacPlayTools` serializes to `"MacPlayTools"`.
- `get_image()` returns PNG `Option<Vec<u8>>`; `get_fresh_image()` = `async_screencap(true)` + `get_image()`.

## Raw `maa_sys::binding` functions needed (not wrapped by `maa-core`)

- `AsstGetImageBgr` — BGR frame capture (exported by the dylib, bound in `maa-sys`, no safe wrapper).
- `AsstGetTasksList` — task queue listing (exported, bound, unwrapped).
- `AsstGetNullSize` — buffer-size helper (exported, bound, unwrapped).

All other needed entry points (`AsstCreateEx`, `AsstAsyncConnect`, `AsstAsyncClick`,
`AsstAsyncScreencap`, `AsstGetImage`, `AsstSetInstanceOption`, `AsstSetUserDir`,
`AsstLoadResource`, `AsstGetVersion`, `AsstSetConnectionExtras`, `AsstGetUUID`,
`AsstBackToHome`, `AsstLog`) have safe `Assistant` wrappers.

## Workspace

- `cargo build` succeeds (rustc 1.97.1). Git dep `maa-cli@aebb5e9` resolved and compiled without workarounds.
- `rmcp` needed the `client` feature added: `transport-streamable-http-client-reqwest` references `RoleClient` which is gated behind `client`; without it rmcp 3.4.0 fails with `unresolved import crate::RoleClient`.
- `jsonschema` pinned `0.56` (v0.56.0 published 2026-09-10; latest stable ≥7 days).
