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
| connect | PASS | `AsstAsyncConnect` 67 ms; MaaCore screencap 30–105 ms; click 230–239 ms |
| playtools-concurrent | PASS | PlayTools VERN=4, SIZE=1920×1080; direct BGR capture works while MaaCore holds its connection (frame arrives as the Retina backing store, 2956×1662 — arkd normalises it to screenshot space) |
| hud-sample | PASS | embedded PlayCover HUD template (⏸ ROI) now ships as the `playtools` default |

## Live numbers (2026-09-19, MaaCore v6.17.5 + PlayCover)

- `SingleStep stage` completes instantly — it only loads the tile map, no navigation.
- `SingleStep start` (BattleStartAll) ≈ 6 s.
- `subtype: "action"` details must be `{"actions": [...]}`; a bare action object
  makes `AsstAppendTask` return 0.
- MaaCore click space (`AsstAsyncClick`) is its normalised 1280×720 frame, not
  device pixels (ControlScaleProxy scales by `device_height/720` for 16:9+).
- MaaCore emits completion callbacks *before* clearing `AsstRunning` — callers
  must wait out a short grace before starting the next step.
- MacBGR: measured. 10-frame MaaCore `Screencap` cost — `General` avg 28.7 ms
  (range 22–43 ms) vs `MacBGR` avg 30.8 ms (range 20–49 ms); colours correct on
  both. No measurable difference on this Mac — the default stays `General`.

## A5 live run (2026-09-19)

`battle start-paused` → `deploy` → `resume_until` against LS-1 on real
MaaCore v6.17.5 + PlayCover:

- `start_paused`: fired on the HUD template (`hud_score` 0.98), `pause_attempts`
  2, 11.4 s end to end.
- `deploy_batch` 塞雷娅 at (7,4): `frame_changed: true` after 4.1 s.
- `resume_until` 12 s: `timeout_paused` — battle resumed and stayed live.
- The abandon dialog showed 理智 +0 — the whole drill ran through the 演习
  entry, costing no sanity.

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

## HUD template

The HUD template is optional. `battle_start_paused` decides when to click pause
by watching the battle HUD area of each frame; a configured template
(`devices[].hud_template`, plus `hud_roi` and `hud_threshold`) lets it fire the
instant the field renders instead of waiting on MaaCore. Without a template the
daemon falls back to MaaCore's own `start` single-step completing together with
a frame that differs from the formation screen.

To capture one later: run `arkd screenshot -o paused.png` on a paused battle
(scale 1.0, png) and set `hud_template = "…/paused.png"`. The file is the whole
frame, not a crop — `HudTemplate::from_png` crops `hud_roi` out of it itself
(default `hud_roi = (1180, 30, 60, 50)` on the 1280x720 screenshot). Adjust
`hud_roi` only if the pause button is not at the default position. Frames that
arrive at a different resolution are scored with the ROI scaled proportionally.
