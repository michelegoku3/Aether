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
REM    3. npm audit             -> block high/critical vulnerabilities
REM    4. npm run verify         -> tsc/vite, ESLint, test/coverage,
REM       wiring frontend, export/dipendenze npm inutilizzati
REM    5. cargo clippy          -> Rust lint (warnings = errors, policy in Cargo.toml)
REM    6. cargo audit + machete -> crate CVEs + unused Cargo dependencies
REM    7. cargo test            -> unit tests + IPC contract tests
REM    8. npm run tauri build   -> compile AetherDesk.exe
REM    9. assemble the portable folder + create the ZIP
REM
REM  The extra Rust tools (cargo-audit, cargo-machete) are installed on first
REM  use with `cargo install` when missing (one time, ~1-2 min).
REM
REM  Output: AetherDesk\build\portable\AetherDesk-<version>.zip
REM
REM  Usage:
REM    build.cmd                        -> full build (tests included)
REM    build.cmd /skipaudit             -> skip npm audit + cargo audit/machete
REM    build.cmd /skiptests             -> skip cargo test (clippy and lint stay)
REM    build.cmd /skipaudit /skiptests  -> build only (flag order is free)
REM
REM  Build output is streamed live through PowerShell Tee-Object (UTF-8 +
REM  forced ANSI colors) while also being captured for the final diagnostic
REM  scan. A clean success closes automatically; warnings/errors stay open.
REM
REM  Notes:
REM    - Tests run BEFORE the release build: when they fail no minutes are
REM      spent compiling an exe that would never ship.
REM    - `cargo test` compiles and runs the same exe as the app, which on Windows
REM      embeds the requireAdministrator manifest: without precautions every run
REM      would pop a UAC prompt (or fail in a non-interactive context such as a
REM      CI). Step 5 therefore sets AETHERDESK_NO_ADMIN_MANIFEST=1 ONLY inside
REM      the child process (build.rs produces an asInvoker exe); step 6
REM      explicitly clears the variable, so the published exe stays elevated
REM      exactly as before.
REM    - Step 4 runs `npm run verify` (including tsc + vite) because `cargo test`
REM      compiles `generate_context!`, which requires dist\ to exist;
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
echo [1/9] Approving esbuild install script (npm 11+)...
call npm install-scripts approve esbuild >nul 2>&1

REM --- Step 2: install dependencies ------------------------------------
echo.
echo [2/9] Installing frontend dependencies (npm ci)...
powershell.exe -NoProfile -ExecutionPolicy Bypass -Command ^
  "$utf8=New-Object System.Text.UTF8Encoding($false); [Console]::OutputEncoding=$utf8; $OutputEncoding=$utf8;" ^
  "& cmd.exe /d /s /c 'npm ci 2>&1' | Tee-Object -FilePath '%BUILD_LOG%' -Append; $code=$LASTEXITCODE; exit $code"
if errorlevel 1 goto :fail

REM --- Step 3: dependency audit (optional, non-mutating) ----------------
if "%SKIP_AUDIT%"=="1" goto :skip_audit
echo.
echo [3/9] Checking high/critical dependency vulnerabilities (npm audit)...
powershell.exe -NoProfile -ExecutionPolicy Bypass -Command ^
  "$utf8=New-Object System.Text.UTF8Encoding($false); [Console]::OutputEncoding=$utf8; $OutputEncoding=$utf8;" ^
  "& cmd.exe /d /s /c 'npm audit --audit-level=high 2>&1' | Tee-Object -FilePath '%BUILD_LOG%' -Append; $code=$LASTEXITCODE; exit $code"
if errorlevel 1 goto :fail
goto :after_audit

:skip_audit
echo.
echo [3/9] Audit skipped.

:after_audit

REM --- Step 4: frontend build + wiring guard ----------------------
echo.
echo [4/9] Verifying Tauri versions + frontend (build, lint, tests, wiring, unused)...
REM  lint:ci runs with --quiet: only ESLint errors show in the build, not the
REM  warnings (still visible via `npm run lint`). Without --quiet every build
REM  would close with "SUCCEEDED WITH WARNINGS" over unactionable noise.
powershell.exe -NoProfile -ExecutionPolicy Bypass -Command ^
  "$utf8=New-Object System.Text.UTF8Encoding($false); [Console]::OutputEncoding=$utf8; $OutputEncoding=$utf8;" ^
  "& cmd.exe /d /s /c 'npm run verify 2>&1' | Tee-Object -FilePath '%BUILD_LOG%' -Append; $code=$LASTEXITCODE; exit $code"
if errorlevel 1 goto :fail

REM --- Step 5: clippy (Rust lint) -------------------------------------------
REM  The lint policy lives in src-tauri/Cargo.toml ([lints.rust]/[lints.clippy]),
REM  so rust-analyzer in the IDE, this script and GitHub all show the same things.
REM  -D warnings: every warning (even rustc dead_code/unused) blocks the build.
REM  --all-targets includes tests, so test code stays clean too.
echo.
echo [5/9] Linting Rust (cargo clippy, warnings are errors)...
cd /d "%DESK_DIR%\src-tauri"
powershell.exe -NoProfile -ExecutionPolicy Bypass -Command ^
  "$utf8=New-Object System.Text.UTF8Encoding($false); [Console]::OutputEncoding=$utf8; $OutputEncoding=$utf8;" ^
  "$env:AETHERDESK_NO_ADMIN_MANIFEST='1';" ^
  "& cmd.exe /d /s /c 'cargo clippy --locked --all-targets --color always -- -D warnings 2>&1' | Tee-Object -FilePath '%BUILD_LOG%' -Append; $code=$LASTEXITCODE; exit $code"
set "CLIPPY_EXIT=%ERRORLEVEL%"
cd /d "%DESK_DIR%"
if not "%CLIPPY_EXIT%"=="0" goto :fail

REM --- Step 6: cargo audit + cargo machete (optional with /skipaudit) ---------
REM  cargo audit   : compares Cargo.lock against the RustSec database (known CVEs).
REM                  The `npm audit` equivalent for crates: zip, unrar, sevenz,
REM                  image, reqwest parse third-party-downloaded input.
REM  cargo machete : dependencies declared in Cargo.toml but never used.
REM  Both are installed on first use when missing.
REM
REM  --ignore: AetherDesk is Windows-only. The two ignored advisories concern
REM  crates of the GTK/webkit stack that tauri declares for the Linux target: they
REM  are NEVER compiled into the Windows exe (they stay in Cargo.lock only because
REM  cargo resolves tauri's full graph).
REM    RUSTSEC-2024-0429  glib (unsound, Linux only)
REM    RUSTSEC-2024-0370  proc-macro-error (unmaintained, glib/gtk macro, Linux only)
if "%SKIP_AUDIT%"=="1" goto :skip_cargo_audit
echo.
echo [6/9] Auditing Rust dependencies (cargo audit + cargo machete)...
cd /d "%DESK_DIR%\src-tauri"
powershell.exe -NoProfile -ExecutionPolicy Bypass -Command ^
  "$utf8=New-Object System.Text.UTF8Encoding($false); [Console]::OutputEncoding=$utf8; $OutputEncoding=$utf8;" ^
  "& cmd.exe /d /s /c 'cargo audit --version >nul 2>&1 || cargo install cargo-audit --locked 2>&1' | Tee-Object -FilePath '%BUILD_LOG%' -Append;" ^
  "& cmd.exe /d /s /c 'cargo machete --version >nul 2>&1 || cargo install cargo-machete --locked 2>&1' | Tee-Object -FilePath '%BUILD_LOG%' -Append;" ^
  "& cmd.exe /d /s /c 'cargo audit --color always --ignore RUSTSEC-2024-0429 --ignore RUSTSEC-2024-0370 2>&1 && cargo machete 2>&1' | Tee-Object -FilePath '%BUILD_LOG%' -Append; $code=$LASTEXITCODE; exit $code"
set "AUDIT_EXIT=%ERRORLEVEL%"
cd /d "%DESK_DIR%"
if not "%AUDIT_EXIT%"=="0" goto :fail
goto :after_cargo_audit

:skip_cargo_audit
echo.
echo [6/9] Cargo audit skipped.

:after_cargo_audit

REM --- Step 7: test (unit + contratto IPC), opzionale ----------------------
if "%SKIP_TESTS%"=="1" goto :skip_tests
echo.
echo [7/9] Running Rust tests (cargo test, non-elevated harness)...
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
echo [7/9] Tests skipped.

:after_tests

REM --- Step 8: compile the binary ---------------------------------------
REM The release build must NOT inherit AETHERDESK_NO_ADMIN_MANIFEST: the
REM published exe embeds the requireAdministrator manifest.
set "AETHERDESK_NO_ADMIN_MANIFEST="
echo.
echo [8/9] Compiling AetherDesk (npm run tauri build)...
powershell.exe -NoProfile -ExecutionPolicy Bypass -Command ^
  "$utf8=New-Object System.Text.UTF8Encoding($false); [Console]::OutputEncoding=$utf8; $OutputEncoding=$utf8;" ^
  "& cmd.exe /d /s /c 'npm run tauri build 2>&1' | Tee-Object -FilePath '%BUILD_LOG%' -Append; $code=$LASTEXITCODE; exit $code"
if errorlevel 1 goto :fail

REM --- Step 9: assemble the portable folder + ZIP -----------------------
echo.
echo [9/9] Assembling portable folder and creating ZIP...
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
