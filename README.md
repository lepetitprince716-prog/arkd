# arkd

Daemon, MCP server and CLI for driving Arknights through MaaCore and
PlayTools. One Rust binary owns the MaaCore instance, the game connection and
the battle loop; every agent, CLI or proxy talks to it over MCP — no client
ever links MaaCore or holds FFI state of its own.

## Architecture

```
Claude Code / Devin / Hermes / Codex / ...
        │  MCP (stdio `arkd mcp`  or  streamable HTTP)
        ▼
arkd serve ── /mcp (streamable HTTP, bearer auth) ── /healthz
        │
        ▼
arkd-core ── MaaCore (loaded at runtime from MAA's dylib)
          ── PlayTools TCP (PlayCover) or adb
```

- The **daemon** owns sessions, task queues, event logs and battle context —
  MaaCore's copilot context only lives in the daemon process, so battles are
  orchestrated server-side.
- The **stdio proxy** (`arkd mcp`) exposes the same MCP endpoint to clients
  that only speak stdio, carrying tokens and custom HTTP headers.
- The **CLI** is a thin MCP client over the same tools.

## Requirements

- An installed MAA (provides `libMaaCore`/`MaaCore.dll` and the `resource`
  tree).
- PlayCover with the MaaTools touch server (port 1717), or an adb emulator
  (e.g. MuMu 12).
- MAA.app must not hold the PlayTools connection while arkd uses it.

## Install

- Prebuilt binaries from the release assets
  (`arkd-<version>-aarch64-apple-darwin.tar.gz`,
  `arkd-<version>-x86_64-pc-windows-msvc.zip`), or
- `cargo install --git https://github.com/lepetitprince716-prog/arkd arkd`

## Quickstart

```sh
arkd doctor                  # checks config, MaaCore, device reachability
arkd token init              # generate the bearer token
arkd serve                   # listens on http://127.0.0.1:7717/mcp
arkd status                  # session status via the daemon
arkd connect                 # connect the default device
arkd screenshot -o shot.png  # capture the game screen
```

Configuration reference: `docs/config.md`. Client setup snippets:
`docs/clients.md`. Remote deployment (launchd, Cloudflare tunnel, Windows
Scheduled Task): `deploy/`.

## Tools

| group | tools |
|---|---|
| infra | `status`, `devices_list`, `device_connect`, `doctor` |
| tasks | `maa_task_types`, `maa_task_schema`, `maa_append`, `maa_update_params`, `maa_queue`, `maa_start`, `maa_stop`, `maa_wait`, `maa_events`, `maa_daily`, `maa_back_home` |
| screen | `screen_capture`, `screen_tap`, `screen_touch`, `screen_drag` |
| battle | `battle_state`, `battle_set_stage`, `battle_start`, `battle_action` |
| combat loop | `battle_start_paused`, `battle_deploy_batch`, `battle_resume_until`, `battle_pause`, `battle_is_paused` |

## The battle loop

Deployments are driven through MaaCore `SingleStep` actions, and the CN client
accepts them while the battle is paused. The intended loop is:

1. `battle_start_paused <stage>` — loads the tile map, starts the fight and
   pauses as soon as the field renders (HUD template, or start-step
   completion as the fallback signal). Returns the paused frame.
2. Inspect the frame; plan deployments.
3. `battle_deploy_batch […]` — deploys each operator while paused; per-step
   errors are reported without aborting the batch.
4. `battle_resume_until --seconds N` — resumes; returns when the screen
   settles or pauses again on timeout. Look, deploy, repeat.

Do not reproduce this with `battle_start` + `screen_tap`: the pause window is
timing-sensitive and the battle context must stay in the daemon process.

## Coordinates

Screenshot space is the normalised `screenshot_size` (default 1280×720) — the
PNG returned by `screen_capture` and the battle tools. `device_size` (e.g.
1920×1080 on PlayCover) is where taps land; the daemon converts between them.
All tools that take coordinates default to screenshot space.

## Development

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --features arkd-core/fake -- -D warnings
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build --release -p arkd
```

License: AGPL-3.0-only (see `LICENSE`). Third-party provenance and vendored
sources: `PROVENANCE.md`.
