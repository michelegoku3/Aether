@echo off
REM ============================================================
REM  build_desk.cmd - Single build script for portable AetherDesk.
REM
REM  Lives at the repository root and works from there: every
REM  path below is resolved relative to this file, so you can
REM  run it from anywhere (double-click or command line).
REM
REM  Steps, all inside AetherDesk\:
REM    1. approve the esbuild install script (npm 11+)
REM    2. npm ci                -> install dependencies (skipped when
REM       package-lock.json is unchanged since the last install)
REM    3. npm audit             -> block high/critical vulnerabilities
REM    4. npm run build, then the frontend gates (npm run verify:build) in
REM       parallel with cargo fmt/clippy and (unless /skiptests) cargo test
REM    5. cargo audit + machete -> crate CVEs + unused Cargo dependencies
REM    6. npm run tauri build   -> compile AetherDesk.exe
REM    7. assemble the portable folder + create the ZIP
REM
REM  The extra Rust tools (cargo-audit, cargo-machete) are installed on first
REM  use with `cargo install` when missing (one time, ~1-2 min).
REM
REM  Output: AetherDesk\build\portable\AetherDesk-<version>.zip
REM
REM  Usage:
REM    build_desk.cmd                        -> full build (tests included)
REM    build_desk.cmd /skipaudit             -> skip npm audit + cargo audit/machete
REM    build_desk.cmd /skiptests             -> skip cargo test (clippy and lint stay)
REM    build_desk.cmd /skipaudit /skiptests  -> build only (flag order is free)
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
REM      CI). Step 4 therefore sets AETHERDESK_NO_ADMIN_MANIFEST=1 ONLY inside
REM      the child process (build.rs produces an asInvoker exe); step 6
REM      explicitly clears the variable, so the published exe stays elevated
REM      exactly as it is shipped.
REM    - Step 4 builds the frontend first because `cargo test` compiles
REM      `generate_context!`, which requires dist\ to exist. Only the gates that
REM      do not touch dist\ then run in parallel with the Rust chain; the
REM      frontend half buffers its output and replays it into the build log, so
REM      a failure stays readable next to the Rust half's streamed output.
REM      `npm run tauri build` rebuilds dist\ through beforeBuildCommand.
REM    - The frontend gates run without coverage (`test:run`, not `test:ci`):
REM      coverage thresholds are a CI gate, not something every local build
REM      needs to pay for.
REM    - `cargo audit` skips the advisory-database fetch when the local database
REM      is younger than a week, so the network cost is paid weekly instead of
REM      on every build.
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
set "CI_STAMP=%DESK_DIR%\node_modules\.package-lock.sha256"
set "AUDIT_DB=%USERPROFILE%\.cargo\advisory-db"
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
REM  npm ci deletes and reinstalls node_modules, which costs more than the
REM  gates it enables. A stamp holding the SHA-256 of package-lock.json
REM  inside node_modules lets unchanged lockfiles skip the whole install.
echo.
echo [2/7] Installing frontend dependencies (npm ci)...
set "LOCK_HASH="
for /f "usebackq delims=" %%h in (`powershell.exe -NoProfile -Command "(Get-FileHash -Algorithm SHA256 -LiteralPath '%DESK_DIR%\package-lock.json').Hash"`) do set "LOCK_HASH=%%h"
set "STAMP_HASH="
if exist "%CI_STAMP%" set /p STAMP_HASH=<"%CI_STAMP%"
if not exist "%DESK_DIR%\node_modules" goto :npm_ci_run
if /i not "%STAMP_HASH%"=="%LOCK_HASH%" goto :npm_ci_run
if "%LOCK_HASH%"=="" goto :npm_ci_run
echo        package-lock.json unchanged since the last install; npm ci skipped.
goto :npm_ci_done

:npm_ci_run
powershell.exe -NoProfile -ExecutionPolicy Bypass -Command ^
  "$utf8=New-Object System.Text.UTF8Encoding($false); [Console]::OutputEncoding=$utf8; $OutputEncoding=$utf8;" ^
  "& cmd.exe /d /s /c 'npm ci 2>&1' | Tee-Object -FilePath '%BUILD_LOG%' -Append; $code=$LASTEXITCODE; exit $code"
if errorlevel 1 goto :fail
if not "%LOCK_HASH%"=="" powershell.exe -NoProfile -Command "Set-Content -LiteralPath '%CI_STAMP%' -Value '%LOCK_HASH%' -NoNewline -Encoding ascii"

:npm_ci_done

REM --- Step 3: dependency audit (optional, non-mutating) ----------------
if "%SKIP_AUDIT%"=="1" goto :skip_audit
echo.
echo [3/7] Checking high/critical dependency vulnerabilities (npm audit)...
powershell.exe -NoProfile -ExecutionPolicy Bypass -Command ^
  "$utf8=New-Object System.Text.UTF8Encoding($false); [Console]::OutputEncoding=$utf8; $OutputEncoding=$utf8;" ^
  "& cmd.exe /d /s /c 'npm audit --audit-level=high 2>&1' | Tee-Object -FilePath '%BUILD_LOG%' -Append; $code=$LASTEXITCODE; exit $code"
if errorlevel 1 goto :fail
goto :after_audit

:skip_audit
echo.
echo [3/7] Audit skipped.

:after_audit

REM --- Step 4: frontend build, then frontend + Rust gates in parallel ---
REM  lint:ci runs with --quiet: only ESLint errors show in the build, not the
REM  warnings (still visible via `npm run lint`). Without --quiet every build
REM  would close with "SUCCEEDED WITH WARNINGS" over unactionable noise.
REM
REM  dist\ is built first because `cargo test` compiles `generate_context!`,
REM  which embeds it; only then do the frontend gates (verify:build) and the
REM  Rust chain run at the same time. They work on different trees, so the
REM  only shared state is the console: the Rust half streams here while the
REM  frontend half buffers its output, which is appended to the build log at
REM  the end and its tail printed when it fails.
REM
REM  The Rust lint policy lives in src-tauri/Cargo.toml ([lints.rust]/
REM  [lints.clippy]), so rust-analyzer, this script and GitHub agree.
REM  -D warnings blocks every warning; --all-targets keeps test code clean too.
REM  cargo fmt runs first (write) and the check right after proves it: the
REM  build normalizes formatting, CI keeps check-only.
echo.
echo [4/7] Building the frontend (tsc + vite)...
powershell.exe -NoProfile -ExecutionPolicy Bypass -Command ^
  "$utf8=New-Object System.Text.UTF8Encoding($false); [Console]::OutputEncoding=$utf8; $OutputEncoding=$utf8;" ^
  "& cmd.exe /d /s /c 'npm run build 2>&1' | Tee-Object -FilePath '%BUILD_LOG%' -Append; $code=$LASTEXITCODE; exit $code"
if errorlevel 1 goto :fail

echo.
echo [4/7] Running frontend gates and Rust lint/tests in parallel...
REM  TEMP/TMP point at the Defender-excluded test dir (see docs/quality-gates.md):
REM  Defender detects the fake Steamless PE files in %TEMP% and returns
REM  os error 225, which would fail these tests on a developer machine.
powershell.exe -NoProfile -ExecutionPolicy Bypass -Command ^
  "$utf8=New-Object System.Text.UTF8Encoding($false); [Console]::OutputEncoding=$utf8; $OutputEncoding=$utf8;" ^
  "$psi=New-Object System.Diagnostics.ProcessStartInfo; $psi.FileName='cmd.exe'; $psi.Arguments='/d /s /c npm run verify:build'; $psi.WorkingDirectory='%DESK_DIR%'; $psi.UseShellExecute=$false; $psi.RedirectStandardOutput=$true; $psi.RedirectStandardError=$true;" ^
  "$p=New-Object System.Diagnostics.Process; $p.StartInfo=$psi; $null=$p.Start(); $stdout=$p.StandardOutput.ReadToEndAsync(); $stderr=$p.StandardError.ReadToEndAsync();" ^
  "Set-Location '%DESK_DIR%\src-tauri';" ^
  "$testTemp=Join-Path $env:LOCALAPPDATA 'Temp\AetherDesk-tests'; New-Item -ItemType Directory -Force -Path $testTemp | Out-Null; $env:TEMP=$testTemp; $env:TMP=$testTemp;" ^
  "$env:AETHERDESK_NO_ADMIN_MANIFEST='1';" ^
  "& cmd.exe /d /s /c 'cargo fmt --all 2>&1 && cargo fmt --all -- --check 2>&1 && cargo clippy --locked --all-targets --color always -- -D warnings 2>&1' | Tee-Object -FilePath '%BUILD_LOG%' -Append; $clippy=$LASTEXITCODE;" ^
  "$tests=0;" ^
  "if ('%SKIP_TESTS%' -eq '1') { Write-Host 'Rust tests skipped.' } else { & cmd.exe /d /s /c 'cargo test --locked --color always 2>&1' | Tee-Object -FilePath '%BUILD_LOG%' -Append; $tests=$LASTEXITCODE }" ^
  "$p.WaitForExit(); $verifyCode=$p.ExitCode; $vout=($stdout.Result + $stderr.Result); $vout | Tee-Object -FilePath '%BUILD_LOG%' -Append | Out-Null;" ^
  "if ($verifyCode -ne 0) { Write-Host ''; Write-Host '--- npm run verify:build (last 60 lines) ---'; ($vout.Split([char]10) | Select-Object -Last 60) | ForEach-Object { Write-Host $_ } }" ^
  "if ($verifyCode -ne 0 -or $clippy -ne 0 -or $tests -ne 0) { exit 1 }"
if errorlevel 1 goto :fail

REM --- Step 5: cargo audit + cargo machete (optional with /skipaudit) ---------
REM  cargo audit   : compares Cargo.lock against the RustSec database (known CVEs).
REM                  The `npm audit` equivalent for crates: zip, unrar, sevenz,
REM                  image, reqwest parse third-party-downloaded input.
REM  cargo machete : dependencies declared in Cargo.toml but never used.
REM  Both are installed on first use when missing.
REM
REM  --no-fetch: refreshing the advisory database costs a network round trip on
REM  every build for data that changes daily at most. The flag is passed while
REM  the local database is younger than a week, so a stale database still
REM  refreshes itself once it ages out.
REM
REM  --ignore: AetherDesk is Windows-only. The two ignored advisories concern
REM  crates of the GTK/webkit stack that tauri declares for the Linux target: they
REM  are NEVER compiled into the Windows exe (they stay in Cargo.lock only because
REM  cargo resolves tauri's full graph).
REM    RUSTSEC-2024-0429  glib (unsound, Linux only)
REM    RUSTSEC-2024-0370  proc-macro-error (unmaintained, glib/gtk macro, Linux only)
if "%SKIP_AUDIT%"=="1" goto :skip_cargo_audit
echo.
echo [5/7] Auditing Rust dependencies (cargo audit + cargo machete)...
cd /d "%DESK_DIR%\src-tauri"
powershell.exe -NoProfile -ExecutionPolicy Bypass -Command ^
  "$utf8=New-Object System.Text.UTF8Encoding($false); [Console]::OutputEncoding=$utf8; $OutputEncoding=$utf8;" ^
  "& cmd.exe /d /s /c 'cargo audit --version >nul 2>&1 || cargo install cargo-audit --locked 2>&1' | Tee-Object -FilePath '%BUILD_LOG%' -Append;" ^
  "& cmd.exe /d /s /c 'cargo machete --version >nul 2>&1 || cargo install cargo-machete --locked 2>&1' | Tee-Object -FilePath '%BUILD_LOG%' -Append;" ^
  "$noFetch = if ((Test-Path '%AUDIT_DB%') -and ((Get-Date) - (Get-Item '%AUDIT_DB%').LastWriteTime).TotalDays -le 7) { '--no-fetch' } else { '' };" ^
  "& cmd.exe /d /s /c ('cargo audit --color always ' + $noFetch + ' --ignore RUSTSEC-2024-0429 --ignore RUSTSEC-2024-0370 2>&1 && cargo machete 2>&1') | Tee-Object -FilePath '%BUILD_LOG%' -Append; $code=$LASTEXITCODE; exit $code"
set "AUDIT_EXIT=%ERRORLEVEL%"
cd /d "%DESK_DIR%"
if not "%AUDIT_EXIT%"=="0" goto :fail
goto :after_cargo_audit

:skip_cargo_audit
echo.
echo [5/7] Cargo audit skipped.

:after_cargo_audit

REM --- Step 6: compile the binary ---------------------------------------
REM The release build must NOT inherit AETHERDESK_NO_ADMIN_MANIFEST: the
REM published exe embeds the requireAdministrator manifest.
set "AETHERDESK_NO_ADMIN_MANIFEST="
echo.
echo [6/7] Compiling AetherDesk (npm run tauri build)...
powershell.exe -NoProfile -ExecutionPolicy Bypass -Command ^
  "$utf8=New-Object System.Text.UTF8Encoding($false); [Console]::OutputEncoding=$utf8; $OutputEncoding=$utf8;" ^
  "& cmd.exe /d /s /c 'npm run tauri build 2>&1' | Tee-Object -FilePath '%BUILD_LOG%' -Append; $code=$LASTEXITCODE; exit $code"
if errorlevel 1 goto :fail

REM --- Step 7: assemble the portable folder + ZIP -----------------------
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
