@echo off
REM ============================================================
REM  build.cmd - Single build script for portable AetherDesk.
REM
REM  Lives at the repository root and works from there: every
REM  path below is resolved relative to this file, so you can
REM  run it from anywhere (double-click or command line).
REM
REM  Steps, all inside AetherDesk\:
REM    1. approve the esbuild install script (npm 11+)
REM    2. npm ci                -> install dependencies
REM    3. npm audit fix         -> fix known vulnerabilities
REM    4. npm run tauri build   -> compile AetherDesk.exe
REM    5. assemble the portable folder + create the ZIP
REM
REM  Output: AetherDesk\build\portable\AetherDesk-<version>.zip
REM
REM  Usage:
REM    build.cmd             -> full build
REM    build.cmd /skipaudit  -> skip the audit step (faster)
REM
REM  The window stays open at the end (pause on success, failure and the
REM  early guard error) so the result is readable when double-clicked.
REM
REM  Notes:
REM    - No multi-line if() blocks (incompatible with files saved
REM      in LF); control flow uses goto labels instead.
REM    - Multi-line continuation (^) is used only for the PowerShell
REM      step; the ^ must stay the very last character of the line.
REM    - No PowerShell execution policy required (it is a .cmd).
REM ============================================================

setlocal

REM --- Paths (all relative to this script's folder) -------------------
set "ROOT=%~dp0"
set "DESK_DIR=%ROOT%AetherDesk"
set "PORTABLE_DIR=%DESK_DIR%\build\portable\AetherDesk"
set "RELEASE_EXE=%DESK_DIR%\src-tauri\target\release\AetherDesk.exe"
set "EXTERNAL_TOOLS=%DESK_DIR%\src-tauri\ExternalTools"
set "DEFAULTS_DIR=%DESK_DIR%\src-tauri\assets\defaults"
set "DATA_CONFIG=%PORTABLE_DIR%\AetherData\config"
set "TAURI_CONF=%DESK_DIR%\src-tauri\tauri.conf.json"

REM --- Guard: the AetherDesk folder must exist ------------------------
if exist "%DESK_DIR%" goto :desk_ok
echo [ERROR] AetherDesk folder not found: %DESK_DIR%
echo         Run this script from the repository root.
echo.
pause
exit /b 1

:desk_ok
cd /d "%DESK_DIR%"

REM --- Step 1: approve the esbuild install script (best-effort) ------
echo.
echo [1/5] Approving esbuild install script (npm 11+)...
call npm install-scripts approve esbuild >nul 2>&1

REM --- Step 2: install dependencies ------------------------------------
echo.
echo [2/5] Installing frontend dependencies (npm ci)...
call npm ci
if errorlevel 1 goto :fail

REM --- Step 3: audit fix (optional) -------------------------------------
if /i "%~1"=="/skipaudit" goto :skip_audit
echo.
echo [3/5] Fixing known vulnerabilities (npm audit fix)...
call npm audit fix
goto :after_audit

:skip_audit
echo.
echo [3/5] Audit skipped.

:after_audit

REM --- Step 4: compile the binary ---------------------------------------
echo.
echo [4/5] Compiling AetherDesk (npm run tauri build)...
call npm run tauri build
if errorlevel 1 goto :fail

REM --- Step 5: assemble the portable folder + ZIP -----------------------
echo.
echo [5/5] Assembling portable folder and creating ZIP...
powershell.exe -NoProfile -ExecutionPolicy Bypass -Command ^
  "$d='%DESK_DIR%';" ^
  "$s='%PORTABLE_DIR%';" ^
  "if(Test-Path $s){Remove-Item -Recurse -Force $s};" ^
  "New-Item -ItemType Directory -Force -Path $s | Out-Null;" ^
  "Copy-Item '%RELEASE_EXE%' (Join-Path $s 'AetherDesk.exe');" ^
  "Copy-Item -Recurse '%EXTERNAL_TOOLS%' (Join-Path $s 'ExternalTools');" ^
  "New-Item -ItemType Directory -Force -Path '%DATA_CONFIG%' | Out-Null;" ^
  "Copy-Item -Recurse -Force (Join-Path '%DEFAULTS_DIR%' '*') '%DATA_CONFIG%';" ^
  "$v=(Get-Content '%TAURI_CONF%' -Raw | ConvertFrom-Json).version;" ^
  "$zip=Join-Path '%DESK_DIR%\build\portable' ('AetherDesk-'+$v+'.zip');" ^
  "if(Test-Path $zip){Remove-Item $zip -Force};" ^
  "Compress-Archive -Path $s -DestinationPath $zip -Force;" ^
  "Write-Host ('ZIP created: '+$zip)"
if errorlevel 1 goto :fail

REM --- Done -------------------------------------------------------------
echo.
echo ============================================
echo   BUILD COMPLETED SUCCESSFULLY
echo ============================================
echo.
echo Press any key to close this window...
endlocal
pause >nul
exit /b 0

:fail
echo.
echo BUILD FAILED.
echo.
echo Press any key to close this window...
endlocal
pause >nul
exit /b 1
