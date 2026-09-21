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
REM    4. npm run build + lint + wiring guard -> tsc/vite (crea dist\), ESLint,
REM       moduli frontend mai importati
REM    5. cargo test            -> unit test + test di contratto IPC
REM    6. npm run tauri build   -> compile AetherDesk.exe
REM    7. assemble the portable folder + create the ZIP
REM
REM  Output: AetherDesk\build\portable\AetherDesk-<version>.zip
REM
REM  Usage:
REM    build.cmd                        -> full build (test inclusi)
REM    build.cmd /skipaudit             -> salta npm audit fix
REM    build.cmd /skiptests             -> salta guardia cablaggio + cargo test
REM    build.cmd /skipaudit /skiptests  -> solo build (ordine dei flag libero)
REM
REM  Build output is streamed live through PowerShell Tee-Object (UTF-8 +
REM  forced ANSI colors) while also being captured for the final diagnostic
REM  scan. A clean success closes automatically; warnings/errors stay open.
REM
REM  Notes:
REM    - I test girano PRIMA del build di release: se falliscono non si
REM      spendono minuti a compilare un exe che non verrebbe pubblicato.
REM    - `cargo test` compila ed esegue lo stesso exe dell'app, che su Windows
REM      embedda il manifest requireAdministrator: senza accorgimenti ogni run
REM      aprirebbe un prompt UAC (o fallirebbe in un contesto non interattivo,
REM      come una CI). Lo step 5 imposta quindi AETHERDESK_NO_ADMIN_MANIFEST=1
REM      SOLO dentro il processo figlio (build.rs produce un exe asInvoker);
REM      lo step 6 azzera esplicitamente la variabile, così l'exe pubblicato
REM      resta elevated esattamente come prima.
REM    - Lo step 4 esegue `npm run build` (tsc + vite) perché `cargo test`
REM      compila `generate_context!`, che richiede l'esistenza di dist\;
REM      `npm run tauri build` lo riesegue da beforeBuildCommand.
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
set "BUILD_LOG=%TEMP%\aether_build_%RANDOM%_%RANDOM%.log"
set "FORCE_COLOR=1"
set "CARGO_TERM_COLOR=always"

REM --- Flags (ordine libero: /skipaudit e /skiptests) -----------------
set "SKIP_AUDIT=0"
set "SKIP_TESTS=0"
if /i "%~1"=="/skipaudit" set "SKIP_AUDIT=1"
if /i "%~2"=="/skipaudit" set "SKIP_AUDIT=1"
if /i "%~1"=="/skiptests" set "SKIP_TESTS=1"
if /i "%~2"=="/skiptests" set "SKIP_TESTS=1"
type nul > "%BUILD_LOG%"

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
echo [1/7] Approving esbuild install script (npm 11+)...
call npm install-scripts approve esbuild >nul 2>&1

REM --- Step 2: install dependencies ------------------------------------
echo.
echo [2/7] Installing frontend dependencies (npm ci)...
powershell.exe -NoProfile -ExecutionPolicy Bypass -Command ^
  "$utf8=New-Object System.Text.UTF8Encoding($false); [Console]::OutputEncoding=$utf8; $OutputEncoding=$utf8;" ^
  "& cmd.exe /d /s /c 'npm ci 2>&1' | Tee-Object -FilePath '%BUILD_LOG%' -Append; $code=$LASTEXITCODE; exit $code"
if errorlevel 1 goto :fail

REM --- Step 3: audit fix (optional) -------------------------------------
if "%SKIP_AUDIT%"=="1" goto :skip_audit
echo.
echo [3/7] Fixing known vulnerabilities (npm audit fix)...
powershell.exe -NoProfile -ExecutionPolicy Bypass -Command ^
  "$utf8=New-Object System.Text.UTF8Encoding($false); [Console]::OutputEncoding=$utf8; $OutputEncoding=$utf8;" ^
  "& cmd.exe /d /s /c 'npm audit fix 2>&1' | Tee-Object -FilePath '%BUILD_LOG%' -Append; $code=$LASTEXITCODE; exit $code"
if errorlevel 1 goto :fail
goto :after_audit

:skip_audit
echo.
echo [3/7] Audit skipped.

:after_audit

REM --- Step 4: frontend build + guardia di cablaggio ----------------------
echo.
echo [4/7] Building frontend (tsc + vite), lint and module wiring...
REM  lint:ci gira con --quiet: in build compaiono solo gli errori ESLint, non le
REM  warning (che restano visibili con `npm run lint`). Senza --quiet ogni build
REM  chiudrebbe con "SUCCEEDED WITH WARNINGS" per rumore non azionabile.
powershell.exe -NoProfile -ExecutionPolicy Bypass -Command ^
  "$utf8=New-Object System.Text.UTF8Encoding($false); [Console]::OutputEncoding=$utf8; $OutputEncoding=$utf8;" ^
  "& cmd.exe /d /s /c 'npm run build 2>&1 && npm run lint:ci 2>&1 && npm run check:wiring 2>&1' | Tee-Object -FilePath '%BUILD_LOG%' -Append; $code=$LASTEXITCODE; exit $code"
if errorlevel 1 goto :fail

REM --- Step 5: test (unit + contratto IPC), opzionale ----------------------
if "%SKIP_TESTS%"=="1" goto :skip_tests
echo.
echo [5/7] Running Rust tests (cargo test, non-elevated harness)...
cd /d "%DESK_DIR%\src-tauri"
powershell.exe -NoProfile -ExecutionPolicy Bypass -Command ^
  "$utf8=New-Object System.Text.UTF8Encoding($false); [Console]::OutputEncoding=$utf8; $OutputEncoding=$utf8;" ^
  "$env:AETHERDESK_NO_ADMIN_MANIFEST='1';" ^
  "& cmd.exe /d /s /c 'cargo test --locked --color always 2>&1' | Tee-Object -FilePath '%BUILD_LOG%' -Append; $code=$LASTEXITCODE; exit $code"
set "TEST_EXIT=%ERRORLEVEL%"
cd /d "%DESK_DIR%"
if not "%TEST_EXIT%"=="0" goto :fail
goto :after_tests

:skip_tests
echo.
echo [5/7] Tests skipped.

:after_tests

REM --- Step 6: compile the binary ---------------------------------------
REM Il build di release NON deve ereditare AETHERDESK_NO_ADMIN_MANIFEST:
REM l'exe pubblicato embedda il manifest requireAdministrator.
set "AETHERDESK_NO_ADMIN_MANIFEST="
echo.
echo [6/7] Compiling AetherDesk (npm run tauri build)...
powershell.exe -NoProfile -ExecutionPolicy Bypass -Command ^
  "$utf8=New-Object System.Text.UTF8Encoding($false); [Console]::OutputEncoding=$utf8; $OutputEncoding=$utf8;" ^
  "& cmd.exe /d /s /c 'npm run tauri build 2>&1' | Tee-Object -FilePath '%BUILD_LOG%' -Append; $code=$LASTEXITCODE; exit $code"
if errorlevel 1 goto :fail

REM --- Step 5: assemble the portable folder + ZIP -----------------------
echo.
echo [7/7] Assembling portable folder and creating ZIP...
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
powershell.exe -NoProfile -ExecutionPolicy Bypass -Command ^
  "$log = if (Test-Path '%BUILD_LOG%') { Get-Content '%BUILD_LOG%' -Raw } else { '' };" ^
  "$plain = $log -replace '\x1b\[[0-9;]*[a-zA-Z]', '';" ^
  "$hasWarn = $plain -match '(?i)(warning:|\bwarning\b|npm warn|deprecated|error:|error\[|npm error)';" ^
  "$hasVuln = ($plain -match '(?i)vulnerabilit') -and ($plain -notmatch '(?i)found 0 vulnerabilities');" ^
  "if ($hasWarn -or $hasVuln) { exit 2 } else { exit 0 }"
if errorlevel 2 goto :success_with_warnings
if exist "%BUILD_LOG%" del /q "%BUILD_LOG%" >nul 2>&1
endlocal
exit /b 0

:success_with_warnings
echo BUILD SUCCEEDED WITH WARNINGS.
echo Review the warning lines above before closing this window.
echo.
echo Press any key to close this window...
if exist "%BUILD_LOG%" del /q "%BUILD_LOG%" >nul 2>&1
endlocal
pause >nul
exit /b 0

:fail
echo.
echo BUILD FAILED.
echo.
echo Press any key to close this window...
if exist "%BUILD_LOG%" del /q "%BUILD_LOG%" >nul 2>&1
endlocal
pause >nul
exit /b 1
