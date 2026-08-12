@echo off
REM ============================================================
REM  build.cmd - UNICO script di build per AetherDesk portabile.
REM
REM  Fa tutto in sequenza:
REM    1. approva lo script di installazione di esbuild (npm 11+)
REM    2. npm ci                -> installa le dipendenze (riproducibile)
REM    3. npm audit fix         -> corregge le vulnerabilita note
REM    4. npm run tauri build   -> compila AetherDesk.exe
REM    5. assembla la cartella portabile + crea lo ZIP
REM
REM  Output:  AetherDesk\build\portable\AetherDesk-<versione>.zip
REM
REM  Uso:
REM    build.cmd             -> build completo
REM    build.cmd /skipaudit  -> salta l'audit (piu' veloce)
REM
REM  Nota: NON usa blocchi if() multi-riga (incompatibili con file
REM  salvati in LF) e non richiede policy di esecuzione (e' un .cmd).
REM ============================================================

setlocal
cd /d "%~dp0..\AetherDesk"

REM --- 1. Approva lo script di installazione di esbuild ---------------------
echo [1/5] npm install-scripts approve esbuild (best-effort)...
call npm install-scripts approve esbuild >nul 2>&1

REM --- 2. Installazione dipendenze -----------------------------------------
echo.
echo [2/5] npm ci (dipendenze frontend)...
call npm ci
if errorlevel 1 goto :fail

REM --- 3. Fix vulnerabilita (opzionale) ------------------------------------
if /i "%~1"=="/skipaudit" goto :skipaudit
echo [3/5] npm audit fix (correzione vulnerabilita)...
call npm audit fix
goto :afteraudit
:skipaudit
echo [3/5] Audit saltato.
:afteraudit

REM --- 4. Compilazione binario ---------------------------------------------
echo [4/5] npm run tauri build (compilazione exe)...
call npm run tauri build
if errorlevel 1 goto :fail

REM --- 5. Assemblaggio + ZIP portabile --------------------------------------
echo [5/5] Assemblaggio cartella portabile + ZIP...
powershell.exe -NoProfile -ExecutionPolicy Bypass -Command "$d='%~dp0..\AetherDesk';$s=Join-Path $d 'build\portable\AetherDesk';if(Test-Path $s){Remove-Item -Recurse -Force $s};New-Item -ItemType Directory -Force -Path $s|Out-Null;Copy-Item (Join-Path $d 'src-tauri\target\release\AetherDesk.exe') (Join-Path $s 'AetherDesk.exe');Copy-Item -Recurse (Join-Path $d 'src-tauri\ExternalTools') (Join-Path $s 'ExternalTools');New-Item -ItemType Directory -Force -Path (Join-Path $s 'AetherData\config')|Out-Null;Copy-Item -Recurse -Force (Join-Path (Join-Path $d 'src-tauri\assets\defaults') '*') (Join-Path $s 'AetherData\config');$v=(Get-Content (Join-Path $d 'src-tauri\tauri.conf.json') -Raw|ConvertFrom-Json).version;$zip=Join-Path (Join-Path $d 'build\portable') ('AetherDesk-'+$v+'.zip');if(Test-Path $zip){Remove-Item $zip -Force};Compress-Archive -Path $s -DestinationPath $zip -Force;Write-Host ('ZIP creato: '+$zip)"
if errorlevel 1 goto :fail

echo.
echo ============================================
echo   BUILD COMPLETATO CON SUCCESSO
echo ============================================
endlocal & exit /b 0

:fail
echo.
echo BUILD FALLITO.
endlocal & exit /b 1
