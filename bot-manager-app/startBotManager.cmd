@echo off
setlocal enabledelayedexpansion
REM Double-clickable launcher for the Bot Manager web app.
REM Runs startBotManager.ps1 (same folder), bypassing PowerShell's script execution policy.
REM On first run it installs dependencies automatically; after that it just starts the servers.
REM
REM It also (one time) opens the Windows Firewall for the UI port so you can control the panel from
REM your phone on the same Wi-Fi, and prints the phone URL at the end.

set "BM_RULE=Bot Manager Vite 5173"
set "BM_PORT=5173"

REM --- One-time: allow inbound TCP 5173 on Private networks so other devices (your phone) can reach
REM     the UI. The read-only existence check runs unelevated, so the UAC prompt only appears the
REM     FIRST time (when the rule is missing). Decline it and the app still starts (LAN access blocked).
powershell -NoProfile -ExecutionPolicy Bypass -Command "if (-not (Get-NetFirewallRule -DisplayName $env:BM_RULE -ErrorAction SilentlyContinue)) { Write-Host 'Opening Windows Firewall for the Bot Manager UI (TCP 5173, Private network). Approve the one-time UAC prompt so your phone can connect...' -ForegroundColor Cyan; try { Start-Process powershell -Verb RunAs -Wait -ArgumentList '-NoProfile','-ExecutionPolicy','Bypass','-Command',('New-NetFirewallRule -DisplayName '''+$env:BM_RULE+''' -Direction Inbound -Action Allow -Protocol TCP -LocalPort '+$env:BM_PORT+' -Profile Private | Out-Null'); Write-Host 'Firewall rule added.' -ForegroundColor Green } catch { Write-Host 'Skipped firewall rule (declined or failed) - phone access over Wi-Fi may be blocked until you allow TCP 5173.' -ForegroundColor Yellow } }"

REM --- The Private-scoped rule only applies on networks classified Private. Home Wi-Fi is often
REM     mis-classified as Public, where Windows blocks inbound regardless. Warn + show the one-line fix.
powershell -NoProfile -ExecutionPolicy Bypass -Command "$pub = @(Get-NetConnectionProfile).Where({ $_.NetworkCategory -eq 'Public' }); if ($pub.Count -gt 0) { Write-Host ''; Write-Host 'WARNING: your active network is set to PUBLIC - Windows Firewall will still block your phone.' -ForegroundColor Yellow; foreach ($p in $pub) { Write-Host ('  Fix (run ONCE in an elevated PowerShell): Set-NetConnectionProfile -InterfaceAlias ''' + $p.InterfaceAlias + ''' -NetworkCategory Private') -ForegroundColor Yellow } }"

REM --- Detect this PC's LAN IPv4 (the up adapter that has a default gateway) for the phone URL. ---
set "BM_LANIP="
for /f "usebackq tokens=* delims=" %%I in (`powershell -NoProfile -Command "$c = @(Get-NetIPConfiguration).Where({ $_.IPv4DefaultGateway -and $_.NetAdapter.Status -eq 'Up' }); if ($c.Count -gt 0) { @($c[0].IPv4Address)[0].IPAddress }"`) do set "BM_LANIP=%%I"

REM --- Start the app (backend + frontend dev servers; opens the browser on this PC). ---
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0startBotManager.ps1"
set "BM_RC=%errorlevel%"

echo.
echo ============================================================
echo  Bot Manager UI
echo    This PC:             http://127.0.0.1:%BM_PORT%
if defined BM_LANIP echo    Phone ^(same Wi-Fi^):  http://!BM_LANIP!:%BM_PORT%
echo ============================================================
echo  No login on this panel - only use the phone URL on trusted Wi-Fi.

REM If startup failed, keep this window open so the error is readable (double-click users).
if %BM_RC% geq 1 (
  echo.
  echo ============================================================
  echo  Startup failed - see the message above.
  echo ============================================================
  pause
)

endlocal
