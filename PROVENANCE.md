# Provenance

Code and data taken from other projects, with what was taken and how it was
changed. Dependencies consumed unmodified through Cargo are listed in
`Cargo.lock` and are not repeated here.

## maa-cli (MaaAssistantArknights/maa-cli)

- Repository: https://github.com/MaaAssistantArknights/maa-cli
- Commit: `aebb5e9e064a18c81f5241b8e83baa5ed10bc385` (2026-08-19)
- License: AGPL-3.0-only
- Used as git dependencies: `maa-sys` (runtime feature), `maa-core`,
  `maa-types`, `maa-ffi-types`. No source vendored at the time of writing; if a
  crate is vendored later, record the paths and modifications here.

## arkauto (lepetitprince716-prog/arkauto)

- Repository: https://github.com/lepetitprince716-prog/arkauto
- License: AGPL-3.0-or-later
- Taken and rewritten in Rust:
  - task catalog (`src/arkauto/catalog.py`): exported once to JSON under
    `crates/arkd-core/catalog/`, then consumed as data
  - callback message ids and significance filter (`messages.py`)
  - session bookkeeping, wait conditions and latency assertions (`session.py`,
    `tests/test_session.py`)
  - copilot battle diagnosis (`BattleState`, `_diagnose`)
  - fake MaaCore used by the test suite (`tests/fake_core.py`)
  - MCP tool descriptions and server instructions (`server.py`), condensed

## arknights-agent-tools (workspace directory, same author)

- `battle.py`: `start_fight_and_pause`, `deploy_batch`, `run_until`, `pause`,
  `is_paused` rewritten as daemon-side battle primitives; the PNG size
  threshold heuristic was replaced by HUD template matching
- `maatools.py`: PlayTools frame protocol (`MAA\0` handshake, `VERN`, `SIZE`,
  `BNDL`, `TUCH`) rewritten; `SCRN` and `BGR` capture added from MaaCore's
  `PlayToolsController.cpp`

## MaaAssistantArknights (MaaCore)

- Repository: https://github.com/MaaAssistantArknights/MaaAssistantArknights
- License: AGPL-3.0-only
- Read for protocol facts only (`src/MaaCore/Controller/PlayToolsController.cpp`,
  `include/AsstCaller.h`); no source copied. The library is loaded at runtime
  from the user's MAA installation.
