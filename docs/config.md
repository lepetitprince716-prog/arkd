# arkd configuration reference

The daemon reads one TOML file. Lookup order: `--config PATH`, then the
`ARKD_CONFIG` env var, then the platform default
(`~/.config/arkd/config.toml` on macOS, `%LOCALAPPDATA%\arkd\config.toml` on
Windows). Missing file → built-in defaults. `~` and `%LOCALAPPDATA%` expand in
all path values.

## `[server]`

| key | default | meaning |
|---|---|---|
| `bind` | `127.0.0.1:7717` | Listen address for `/mcp` and `/healthz`. A non-loopback bind without a token refuses to start. Overridden by `ARKD_BIND` / `--bind`. |
| `token_file` | `~/.config/arkd/token` | File holding the bearer token; `ARKD_TOKEN` env wins over it. `arkd token init` generates one. |
| `max_events` | `2000` | Per-session event ring buffer size. |
| `require_token_on_loopback` | `false` | When `true`, the bearer check applies to every request including loopback — required for Cloudflare-tunnel deployments, where the tunnel peer arrives on loopback. `/healthz` stays exempt. |

## `[maa]`

| key | default (macOS) | meaning |
|---|---|---|
| `core_dir` | `/Applications/MAA.app/Contents/Frameworks` | Directory containing the MaaCore dynamic library. `ARKD_MAA_CORE_DIR` overrides. |
| `resource_dir` | `/Applications/MAA.app/Contents/Resources` | Directory containing the `resource` tree loaded by MaaCore. `ARKD_MAA_RESOURCE_DIR` overrides. |
| `user_dir` | `~/.local/state/arkd` | MaaCore user data (logs, cache); also where `launchd print` points log paths. `ARKD_MAA_USER_DIR` overrides. |
| `incremental` | `[]` | Extra resource packs loaded incrementally (`AsstLoadResource`). |

## `[[devices]]`

| key | default | meaning |
|---|---|---|
| `name` | — | Unique device name; `default = true` marks the implicit target. |
| `kind` | `playtools` | `playtools` (PlayCover/MacPlayTools TCP) or `adb`. |
| `address` | `127.0.0.1:1717` | PlayTools TCP endpoint or adb address. |
| `adb_path` | — | (adb only) adb executable path. |
| `touch_mode` | `maatouch` / `MacPlayTools` | (adb only) MaaCore touch mode. |
| `connect_config` | `General` | MaaCore connection config preset (e.g. `MuMuEmulator12`). |
| `screenshot_size` | `[1280, 720]` | Normalised screenshot space all screenshot coordinates refer to. |
| `device_size` | playtools size / `screenshot_size` | Physical device pixels; screenshots scale to it for taps. |
| `pause_button_screenshot` | `[1210, 55]` | Battle pause button in screenshot space (→ device `(1815, 82)` at 1920×1080). |
| `client_type` | `Official` | MaaCore client flavour. |
| `hud_template` | none | Optional PNG used to detect the battle HUD early in `battle_start_paused`. The file is the whole frame; the ROI below is cropped out of it. |
| `hud_roi` | `[1180, 30, 60, 50]` | `(x, y, w, h)` region of the screenshot that carries the HUD template. |
| `hud_threshold` | `0.85` | Normalised-correlation score at or above which the HUD counts as matched. |

## `job_dir` (top level)

Optional sandbox root for file paths written by tools (`screen_capture
save_to`, task file params). Paths must resolve inside it; unset allows any
path. `ARKD_JOB_DIR` overrides.

## Environment variables

| variable | used by | meaning |
|---|---|---|
| `ARKD_CONFIG` | all | Config file path. |
| `ARKD_BIND` | serve | Override `server.bind`. |
| `ARKD_TOKEN` | serve + clients | Bearer token (beats `token_file`). |
| `ARKD_TOKEN_FILE` | serve | Override `server.token_file`. |
| `ARKD_URL` | clients | Daemon MCP endpoint for the CLI / `arkd mcp` (default `http://127.0.0.1:7717/mcp`). |
| `ARKD_HEADERS` | clients | `;`-separated `Name: value` headers sent on every request (e.g. Cloudflare Access service tokens). |
| `ARKD_JOB_DIR` | serve | Override `job_dir`. |
| `ARKD_MAA_CORE_DIR` / `ARKD_MAA_RESOURCE_DIR` / `ARKD_MAA_USER_DIR` | serve | Override the `[maa]` paths. |
