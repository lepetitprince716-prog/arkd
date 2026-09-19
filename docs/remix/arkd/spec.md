# arkd — specification

## Product shape

A single Rust binary, `arkd`, with three roles:

- `arkd serve` — long-lived daemon. Sole owner of MaaCore instances and of the
  optional PlayTools direct socket. Exposes MCP over streamable HTTP at `/mcp`
  plus `GET /healthz`.
- `arkd mcp` — stdio MCP proxy that forwards to a running daemon. For clients
  that only speak stdio.
- `arkd <subcommand>` — CLI built on the same MCP client; no separate REST API.

Callers: MCP clients (Claude Code, Devin, pi, Hermes, Grok CLI) on this machine,
on the LAN, or over a Cloudflare Tunnel. One daemon per host that runs the game
(Mac with PlayCover, Z3 with MuMu); clients choose the daemon by URL.

## Hard constraints

- Language: Rust (edition 2024, toolchain 1.97 locally; MSRV follows `maa-cli`).
- MaaCore is loaded at runtime via `maa-sys`/`maa-core` from the official
  `maa-cli` repository (git dependency pinned to a commit). No compile-time link.
- License: AGPL-3.0-only (matches MaaCore and maa-cli).
- Runs on macOS arm64 (primary) and Windows x86_64 (Z3). Linux is out of scope.
- The battle context lives inside the MaaCore process: every latency-sensitive
  combat loop runs inside the daemon, never one MCP round trip per step.
- Non-loopback requests require `Authorization: Bearer <token>`. Loopback is
  trusted. Internet exposure goes through Cloudflare Tunnel + Access.
- Coordinates accepted by tools default to screenshot space (the PNG the caller
  saw); conversion to device space happens inside the daemon.
- Parameters that spend account resources (`stone`, `medicine`) default to 0
  and are never raised implicitly.

## Required capabilities

1. Device registry from `config.toml`; lazy connect; per-device action
   serialization; `status` reports `busy`.
2. MaaCore task queue: list task types, per-type JSON Schema, validated append,
   update params, queue listing, start, stop, back-to-home, daily routine.
3. Event log (ring buffer) and blocking `wait` with conditions `all_done`,
   `task_done`, `error`, `any_event`, `idle`, `battle_problem`, `battle_stalled`;
   wakes on the MaaCore callback, not on a poll interval; emits MCP progress.
4. Screen: capture (MaaCore or PlayTools direct), tap, raw touch phase,
   polyline drag (PlayTools only); `scale`, `format`, `quality`, `save_to`.
5. Battle: state diagnosis, SingleStep stage/start/action, and the compound
   primitives `battle_start_paused`, `battle_deploy_batch`,
   `battle_resume_until`, `battle_pause`, `battle_is_paused`.
6. `doctor`: library load, resource load, PlayTools handshake and size, port
   listening, MAA.app running warning.
7. Deployment artifacts: launchd plist template, cloudflared config template,
   Windows scheduled-task notes, client configuration snippets, CI and release
   workflows producing macOS arm64 and Windows x86_64 binaries.

## Explicitly out of scope (v1)

- Driving the MAA GUI or implementing MAA's remote-control polling protocol.
- Control leases between agents (only per-device serialization and `busy`).
- Launching or foregrounding the game or emulator.
- Federation: one daemon proxying to another.
- A REST API separate from MCP.
- Installing or updating MaaCore and resources.
- Linux builds.

## Acceptance

- A1 `cargo fmt --check`, `cargo clippy --all-targets -D warnings`, `cargo test`
  pass without MaaCore present; CI green on macOS and Windows.
- A2 `arkd doctor` on the development Mac: library loaded, resources loaded,
  PlayTools handshake OK, size 1920x1080, MAA.app not running.
- A3 From Claude Code over loopback: `status`, `screen_capture` returns a
  1280x720 PNG, `screen_tap` in screenshot coordinates opens an operator card.
- A4 Every tool of the retired Python server has a counterpart; `maa_wait`
  returns within 3 s of a callback fired at 0.2 s (asserted in tests).
- A5 LS-1 drill: `battle_start_paused` returns `paused=true` with a frame;
  `battle_deploy_batch` for one operator reports `frame_changed=true`;
  `battle_resume_until` returns `settled` or `timeout_paused`; sanity delta 0.
- A6 From Z3: `/healthz` returns 200 with the token and 401 without; a phone
  Hermes session completes `screen_capture` through the tunnel.
- A7 Daemon on Z3 connects to MuMu; a client on the Mac retrieves a screenshot.
- A8 PlayTools direct capture works while a MaaCore task is running, or the
  spike documents that MaaTools accepts a single connection and the tool
  degrades with a clear error.
- A9 Repository contains no conversation residue; code carries no comments
  except FFI safety notes.
