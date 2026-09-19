# Connecting MCP clients to arkd

Three transport shapes, in order of preference:

- **stdio proxy** — the client spawns `arkd mcp`, which forwards everything to
  the daemon over HTTP. Works everywhere, carries tokens and custom headers.
- **direct HTTP** — the client speaks streamable HTTP to `/mcp` itself. Loopback
  needs no token; anything else needs `Authorization: Bearer <token>`.
- **tunnel** — the daemon sits behind `cloudflared` + Cloudflare Access; see
  `deploy/cloudflared/README.md`. Clients send `CF-Access-Client-Id` /
  `CF-Access-Client-Secret` headers, plus the bearer when
  `server.require_token_on_loopback` is enabled.

Everywhere a bearer or extra headers are needed but the client cannot send
HTTP headers, use the stdio proxy form — `arkd mcp` accepts `--url`,
`--token-file`, `--header 'Name: value'` and the `ARKD_HEADERS` env var.

## Claude Code (`~/.claude.json`, `mcpServers`)

Loopback, via the stdio proxy:

```json
"arkd": {
  "type": "stdio",
  "command": "arkd",
  "args": ["mcp"]
}
```

LAN, direct HTTP with a bearer token:

```json
"arkd": {
  "type": "http",
  "url": "http://192.168.50.10:7717/mcp",
  "headers": {
    "Authorization": "Bearer <token>"
  }
}
```

Tunnel:

```json
"arkd": {
  "type": "http",
  "url": "https://arkd.<domain>/mcp",
  "headers": {
    "CF-Access-Client-Id": "<id>",
    "CF-Access-Client-Secret": "<secret>",
    "Authorization": "Bearer <token>"
  }
}
```

## Devin (`~/.config/devin/mcp_config.json`)

Loopback via the stdio proxy:

```json
"arkd": {
  "command": "arkd",
  "args": ["mcp"]
}
```

Remote or tunnelled — keep the daemon off the config and let the proxy carry
credentials:

```json
"arkd": {
  "command": "arkd",
  "args": [
    "mcp",
    "--url", "https://arkd.<domain>/mcp",
    "--token-file", "~/.config/arkd/token",
    "--header", "CF-Access-Client-Id: <id>",
    "--header", "CF-Access-Client-Secret: <secret>"
  ]
}
```

## Hermes (`~/.hermes/config.yaml`, `mcp_servers:`)

Loopback via the stdio proxy:

```yaml
mcp_servers:
  arkd:
    command: arkd
    args: [mcp]
    enabled: true
```

LAN or tunnel, direct HTTP (Hermes supports `headers:`):

```yaml
mcp_servers:
  arkd:
    url: https://arkd.<domain>/mcp
    enabled: true
    headers:
      CF-Access-Client-Id: <id>
      CF-Access-Client-Secret: <secret>
      Authorization: Bearer <token>
```

A Hermes gateway running on the same Mac reaches arkd over loopback — no
tunnel needed on that path; the `url: http://127.0.0.1:7717/mcp` form without
headers is enough.

## pi

No documented MCP server configuration was found for pi on this machine
(`~/.pi/agent/settings.json` carries no MCP section). Check pi's own docs for
its MCP config shape; when it supports stdio servers, `arkd mcp` is the entry
point.

## Codex (`~/.codex/config.toml`)

Codex MCP entries are stdio only:

```toml
[mcp_servers.arkd]
command = "arkd"
args = ["mcp"]
```

For a remote daemon, extend the args:

```toml
[mcp_servers.arkd]
command = "arkd"
args = [
  "mcp",
  "--url", "http://192.168.50.10:7717/mcp",
  "--token-file", "~/.config/arkd/token",
]
```

## Grok CLI (`~/.grok/config.toml`)

Grok's `mcp_servers` table is stdio only:

```toml
[mcp_servers.arkd]
command = "arkd"
args = ["mcp"]
```

Use the same `--url` / `--token-file` / `--header` args as the Codex example
for LAN or tunnelled daemons.
