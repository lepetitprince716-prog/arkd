# Exposing arkd through a Cloudflare tunnel

## Important: the bearer token is not enforced on the tunnel path by default

cloudflared connects to the daemon over loopback TCP, so arkd's auth middleware
sees a loopback peer and lets the request through without a bearer token. That
means **Cloudflare Access is the only gate on the tunnel path** — do not skip
step 4.

Alternative: keep the daemon's own bearer enforced even for loopback peers by
setting `server.require_token_on_loopback = true` in the config. With that
switch, clients must send both the Cloudflare Access headers *and*
`Authorization: Bearer <token>`.

## Steps

1. Create the tunnel:

   ```sh
   cloudflared tunnel create arkd
   ```

   This prints a tunnel id and writes `~/.cloudflared/<TUNNEL_ID>.json`.

2. Copy `arkd.yml` to `~/.cloudflared/arkd.yml` and fill in `<TUNNEL_ID>`,
   `<YOU>` and `<DOMAIN>`.

3. Route DNS:

   ```sh
   cloudflared tunnel route dns arkd arkd.<DOMAIN>
   ```

4. In the Cloudflare Zero Trust dashboard, create a **self-hosted Access
   application** for `arkd.<DOMAIN>` with a **service token** policy. Note the
   `CF-Access-Client-Id` / `CF-Access-Client-Secret` pair — clients send both
   as HTTP headers on every request.

5. Install the LaunchAgent (`com.arkd.tunnel.plist` — replace `<YOU>` first):

   ```sh
   cp com.arkd.tunnel.plist ~/Library/LaunchAgents/com.arkd.tunnel.plist
   launchctl bootstrap gui/$(id -u) ~/Library/LaunchAgents/com.arkd.tunnel.plist
   ```

6. Verify:

   ```sh
   curl -s https://arkd.<DOMAIN>/healthz   # 200 once Access accepts the request
   ```

## Client side

- HTTP clients (Claude Code, Hermes, custom): send
  `CF-Access-Client-Id`, `CF-Access-Client-Secret` and, when
  `require_token_on_loopback` is on, `Authorization: Bearer <token>`.
- `arkd` CLI and the `arkd mcp` stdio proxy: pass
  `--header 'CF-Access-Client-Id: <id>' --header 'CF-Access-Client-Secret: <secret>'`,
  or set `ARKD_HEADERS='CF-Access-Client-Id: <id>;CF-Access-Client-Secret: <secret>'`.
