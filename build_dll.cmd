@echo off
REM ============================================================
REM  build_dll.cmd - Single build script for AetherDLL (C++ / CMake).
REM
REM  Twin of build_desk.cmd (AetherDesk): same conventions, because cmd.exe
REM  loses the first character after label jumps in LF files:
REM  no multi-line if() blocks, control flow with goto labels only.
REM
REM  Steps, all inside AetherDLL\:
REM    1. locate VS (vswhere), load the x64 native environment (vcvars)
REM    2. clang-format -i on all sources, then --dry-run --Werror (style gate)
REM    3. cmake --preset x64-Release (Quality.cmake: /W4 /WX, clang-tidy
REM       when present, orphan-header check, unit tests in ALL)
REM    4. cmake --build --preset x64-Release
REM    5. ctest (explicit report)
REM    6. assemble the portable folder + create the ZIP (3 DLLs only)
REM
REM  Output: AetherDLL\build\portable\AetherDLL-<version>.zip
REM  Version is read from CMakeLists.txt project(... VERSION x.y.z).
REM
REM  Usage:
REM    build_dll.cmd                        -> full build (format check + tests)
REM    build_dll.cmd /skipformat            -> skip the clang-format gate
REM    build_dll.cmd /skiptests             -> skip ctest (build still runs tests via ALL)
REM
REM  Notes:
REM    - Needs Visual Studio 2025 (v18) with "Desktop C++" workload.
REM      cmake, Ninja and clang-format are taken from the VS installation,
REM      so no extra PATH setup is required.
REM    - AETHER_WARNINGS_AS_ERRORS=OFF and -DAETHER_CLANG_TIDY=OFF exist
REM      for one-time local triage only; this script never sets them.
REM ============================================================

setlocal

REM --- Paths (all relative to this script's folder) -------------------
set "ROOT=%~dp0"
set "DLL_DIR=%ROOT%AetherDLL"
set "BUILD_LOG=%TEMP%\aetherdll_build_%RANDOM%_%RANDOM%.log"
set "FORCE_COLOR=1"

REM --- Flags (free order: /skipformat and /skiptests) -----------------
set "SKIP_FORMAT=0"
set "SKIP_TESTS=0"
if /i "%~1"=="/skipformat" set "SKIP_FORMAT=1"
if /i "%~2"=="/skipformat" set "SKIP_FORMAT=1"
if /i "%~1"=="/skiptests" set "SKIP_TESTS=1"
if /i "%~2"=="/skiptests" set "SKIP_TESTS=1"
type nul > "%BUILD_LOG%"

REM --- Guard: the AetherDLL folder must exist -------------------------
if exist "%DLL_DIR%" goto :dll_ok
echo [ERROR] AetherDLL folder not found: %DLL_DIR%
echo         Run this script from the repository root.
echo.
pause
exit /b 1

:dll_ok
cd /d "%DLL_DIR%"

REM --- Step 1: Visual Studio environment --------------------------------
echo.
echo [1/6] Locating Visual Studio and loading the x64 environment...
set "VSWHERE=%ProgramFiles(x86)%\Microsoft Visual Studio\Installer\vswhere.exe"
set "VS_DIR="
if exist "%VSWHERE%" goto :vswhere_run
goto :vs_fallback

:vswhere_run
for /f "usebackq delims=" %%i in (`"%VSWHERE%" -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath`) do set "VS_DIR=%%i"
if defined VS_DIR goto :vs_found

:vs_fallback
if exist "C:\Program Files\Microsoft Visual Studio\18\Community" set "VS_DIR=C:\Program Files\Microsoft Visual Studio\18\Community"
if defined VS_DIR goto :vs_found
echo [ERROR] No Visual Studio with C++ tools found. Install VS 2025 (Desktop C++ workload).
echo.
pause
exit /b 1

:vs_found
echo        VS: %VS_DIR%
call "%VS_DIR%\VC\Auxiliary\Build\vcvars64.bat" >nul 2>&1
if errorlevel 1 goto :fail
set "VS_CMAKE=%VS_DIR%\Common7\IDE\CommonExtensions\Microsoft\CMake\CMake\bin\cmake.exe"
set "VS_NINJA_DIR=%VS_DIR%\Common7\IDE\CommonExtensions\Microsoft\CMake\Ninja"
set "VS_CLANG_FORMAT=%VS_DIR%\VC\Tools\Llvm\x64\bin\clang-format.exe"
if exist "%VS_CMAKE%" set "CMAKE_EXE=%VS_CMAKE%"
if not defined CMAKE_EXE set "CMAKE_EXE=cmake"
if exist "%VS_NINJA_DIR%\ninja.exe" set "PATH=%VS_NINJA_DIR%;%PATH%"

REM --- Step 2: clang-format gate (optional) ------------------------------
REM  The build normalizes formatting first (-i in place), the --dry-run check
REM  right after proves it. CI keeps check-only (it cannot commit back fixes).
if "%SKIP_FORMAT%"=="1" goto :skip_format
echo.
echo [2/6] Formatting + checking C++ (clang-format, .clang-format at AetherDLL root)...
if not exist "%VS_CLANG_FORMAT%" goto :no_clang_format
powershell.exe -NoProfile -ExecutionPolicy Bypass -Command ^
  "$c='%VS_CLANG_FORMAT%';" ^
  "$files=@(Get-ChildItem -Path '.' -Include *.cpp,*.h -Recurse | Where-Object { $_.FullName -notmatch '\\out\\' -and $_.FullName -notmatch '\\build' -and $_.FullName -notmatch '\\generated\\' -and $_.FullName -notmatch '\\_deps\\' });" ^
  "foreach ($f in $files) { & $c -i --style=file $f.FullName 2>$null };" ^
  "$bad=@($files | Where-Object { & $c --dry-run --Werror --style=file $_.FullName 2>$null; $LASTEXITCODE -ne 0 });" ^
  "if ($bad.Count -gt 0) { $bad | ForEach-Object { Write-Output ('  unformatted: ' + $_.FullName) } | Tee-Object -FilePath '%BUILD_LOG%' -Append; exit 1 }" ^
  "Write-Output 'clang-format clean.' | Tee-Object -FilePath '%BUILD_LOG%' -Append" ^
  "2>&1"
if errorlevel 1 goto :fail
goto :after_format

:no_clang_format
echo        WARNING: clang-format not found (install 'C++ Clang tools for Windows'). Format gate skipped.
goto :after_format

:skip_format
echo.
echo [2/6] Format check skipped.

:after_format

REM --- Step 3: configure ---------------------------------------------------
REM -Wno-deprecated below is a CMake CLI flag (silences CMake deprecation
REM warnings from old FetchContent dependencies). Third-party compiler noise
REM (minhook, protobuf) is silenced separately via /external:W0 and /w
REM in cmake/Quality.cmake and CMakeLists.txt, not here.
echo.
echo [3/6] Configuring (cmake --preset x64-Release: /W4 /WX, clang-tidy, orphan headers)...
powershell.exe -NoProfile -ExecutionPolicy Bypass -Command ^
  "$utf8=New-Object System.Text.UTF8Encoding($false); [Console]::OutputEncoding=$utf8; $OutputEncoding=$utf8;" ^
  "& '%CMAKE_EXE%' -Wno-deprecated --preset x64-Release 2>&1 | Tee-Object -FilePath '%BUILD_LOG%' -Append; $code=$LASTEXITCODE; exit $code"
if errorlevel 1 goto :fail

REM --- Step 4: build (unit tests compile as part of ALL) --------------------
echo.
echo [4/6] Building (cmake --build --preset x64-Release)...
powershell.exe -NoProfile -ExecutionPolicy Bypass -Command ^
  "$utf8=New-Object System.Text.UTF8Encoding($false); [Console]::OutputEncoding=$utf8; $OutputEncoding=$utf8;" ^
  "& '%CMAKE_EXE%' --build --preset x64-Release 2>&1 | Tee-Object -FilePath '%BUILD_LOG%' -Append; $code=$LASTEXITCODE; exit $code"
if errorlevel 1 goto :fail

REM --- Step 5: unit tests (optional, explicit report) -----------------------
if "%SKIP_TESTS%"=="1" goto :skip_tests
echo.
echo [5/6] Running C++ unit tests (ctest)...
powershell.exe -NoProfile -ExecutionPolicy Bypass -Command ^
  "$utf8=New-Object System.Text.UTF8Encoding($false); [Console]::OutputEncoding=$utf8; $OutputEncoding=$utf8;" ^
  "& cmd.exe /d /s /c 'ctest --test-dir out/build/x64-Release --output-on-failure -C Release 2>&1' | Tee-Object -FilePath '%BUILD_LOG%' -Append; $code=$LASTEXITCODE; exit $code"
if errorlevel 1 goto :fail
goto :after_tests

:skip_tests
echo.
echo [5/6] Tests skipped.

:after_tests

REM --- Step 6: package the 3 DLLs -------------------------------------------
echo.
echo [6/6] Packaging AetherDLL binaries...
powershell.exe -NoProfile -ExecutionPolicy Bypass -Command ^
  "$src='out/build/x64-Release/out/Release';" ^
  "if(!(Test-Path $src)){throw \"DLL output folder not found: $src\"};" ^
  "$required=@('AetherCore.dll','AetherPayload.dll','xinput1_4.dll');" ^
  "$staging='build/portable/AetherDLL';" ^
  "if(Test-Path $staging){Remove-Item -Recurse -Force $staging};" ^
  "New-Item -ItemType Directory -Force -Path $staging | Out-Null;" ^
  "foreach($n in $required){$b=Join-Path $src $n; if(!(Test-Path -LiteralPath $b -PathType Leaf)){throw \"Missing required file: $b\"}; Copy-Item -LiteralPath $b -Destination (Join-Path $staging $n)};" ^
  "$ver=(Select-String -LiteralPath 'CMakeLists.txt' -Pattern 'project\(AetherDLL LANGUAGES C CXX RC VERSION ([0-9.]+)\)').Matches[0].Groups[1].Value;" ^
  "$zip=\"build/portable/AetherDLL-$ver.zip\";" ^
  "if(Test-Path $zip){Remove-Item $zip -Force};" ^
  "Compress-Archive -Path \"$staging/*\" -DestinationPath $zip -Force;" ^
  "Write-Output \"Packaged $zip (version $ver)\" | Tee-Object -FilePath '%BUILD_LOG%' -Append" ^
  "2>&1"
if errorlevel 1 goto :fail

echo.
echo [OK] AetherDLL build complete.
exit /b 0

:fail
echo.
echo [FAIL] AetherDLL build failed. See %BUILD_LOG%.
echo.
pause
exit /b 1
