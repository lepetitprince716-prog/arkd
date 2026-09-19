# Windows deployment (MuMu 12)

1. Download the release zip (`arkd-<version>-x86_64-pc-windows-msvc.zip`) and
   unpack it, e.g. to `C:\arkd\`.

2. Install MAA's core files (the `C:\MAA` layout in `config.example.toml`
   expects `MaaCore.dll` and the `resource` directory under `C:\MAA`) and the
   bundled adb platform-tools.

3. Copy `config.example.toml` to `%LOCALAPPDATA%\arkd\config.toml` and adjust
   paths if your MAA install lives elsewhere. The bundled device `mumu`
   targets MuMu 12's adb endpoint `127.0.0.1:16384` with
   `connect_config = "MuMuEmulator12"`.

4. Sanity checks:

   ```powershell
   C:\arkd\arkd.exe doctor
   C:\arkd\arkd.exe token init
   ```

5. Allow inbound access from your LAN only (adjust the subnet):

   ```powershell
   netsh advfirewall firewall add rule name="arkd" dir=in action=allow protocol=TCP localport=7717 remoteip=192.168.50.0/24
   ```

   The example config binds `0.0.0.0:7717`, so a token is mandatory — the
   daemon refuses to start on a non-loopback bind without one.

6. Register the Scheduled Task (restarts on failure, runs at logon):

   ```powershell
   powershell -File .\install-task.ps1 -ArkdPath C:\arkd\arkd.exe
   Start-ScheduledTask -TaskName arkd
   ```

7. Verify from another host. `/healthz` needs no token; for an authenticated
   check use the CLI:

   ```sh
   curl http://<host>:7717/healthz
   arkd status --url http://<host>:7717/mcp --token-file ~/.config/arkd/token
   ```
