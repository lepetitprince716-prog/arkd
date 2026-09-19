# Registers a Scheduled Task "arkd" that runs `arkd.exe serve` at user logon
# and restarts it on failure every minute. Idempotent: re-running replaces the
# task. PowerShell 5.1 compatible.
#
# Usage:
#   .\install-task.ps1 [-ArkdPath C:\arkd\arkd.exe]
# Manage afterwards:
#   Start-ScheduledTask -TaskName arkd
#   Stop-ScheduledTask  -TaskName arkd
#   Unregister-ScheduledTask -TaskName arkd -Confirm:$false

param(
    [string]$ArkdPath = "$PSScriptRoot\arkd.exe"
)

$ErrorActionPreference = 'Stop'

if (-not (Test-Path $ArkdPath)) {
    Write-Error "arkd.exe not found at $ArkdPath"
}

$action = New-ScheduledTaskAction -Execute $ArkdPath -Argument 'serve' `
    -WorkingDirectory (Split-Path $ArkdPath)

$trigger = New-ScheduledTaskTrigger -AtLogOn

$settings = New-ScheduledTaskSettingsSet `
    -RestartCount 999 `
    -RestartInterval (New-TimeSpan -Minutes 1) `
    -ExecutionTimeLimit ([TimeSpan]::Zero) `
    -AllowStartIfOnBatteries `
    -DontStopIfGoingOnBatteries

Register-ScheduledTask -TaskName 'arkd' -Action $action -Trigger $trigger `
    -Settings $settings -Force | Out-Null

Write-Host "Scheduled task 'arkd' registered for $ArkdPath serve"
Write-Host "Start it now:  Start-ScheduledTask -TaskName arkd"
Write-Host "Stop it:       Stop-ScheduledTask -TaskName arkd"
Write-Host "Remove it:     Unregister-ScheduledTask -TaskName arkd -Confirm:`$false"
