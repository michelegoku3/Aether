@echo off
setlocal EnableExtensions
REM ============================================================
REM regen_proto.cmd - Rigenera AetherDLL/proto/generated/steam_messages.pb.{h,cc}
REM
REM Quando serve: SOLO dopo aver modificato AetherDLL/proto/steam_messages.proto
REM (con AETHER_PROTO_PREGEN=ON, default, i file .pb sono pre-generati e
REM  committati nel repo: protoc non viene nemmeno compilato).
REM
REM Requisito: protoc DEVE essere la versione 25.3 (stessa del runtime
REM protobuf v25.3 usato da FetchContent).
REM ============================================================

set "ROOT=%~dp0.."
set "PROTO_DIR=%ROOT%\AetherDLL\proto"
set "OUT_DIR=%PROTO_DIR%\generated"
set "PROTOC=%ROOT%\AetherDLL\out\build\x64-Release\_deps\protobuf-build\protoc.exe"

if exist "%PROTOC%" goto :run

REM Fallback: un protoc in PATH (verifica tu che sia 25.3)
where protoc >nul 2>nul
if %errorlevel%==0 (
    echo [AVVISO] Uso il protoc trovato in PATH: DEVE essere la versione 25.3.
    protoc --version
    set "PROTOC=protoc"
    goto :run
)

echo [ERRORE] protoc.exe non trovato.
echo.
echo  Con AETHER_PROTO_PREGEN=ON protoc non viene compilato. Alternative:
echo   1. configura una build con -DAETHER_PROTO_PREGEN=OFF, compila una
echo      volta sola il target protoc, poi riesegui questo script;
echo   2. oppure scarica protoc 25.3 da:
echo      https://github.com/protocolbuffers/protobuf/releases/tag/v25.3
echo      ^(protoc-25.3-win64.zip^), mettilo in PATH e riesegui.
echo.
pause
exit /b 1

:run
if not exist "%OUT_DIR%" mkdir "%OUT_DIR%"
"%PROTOC%" --cpp_out=lite:"%OUT_DIR%" -I "%PROTO_DIR%" "%PROTO_DIR%\steam_messages.proto"
if errorlevel 1 (
    echo [ERRORE] protoc ha fallito.
    pause
    exit /b 1
)
echo.
echo Fatto: %OUT_DIR%\steam_messages.pb.h e .cc rigenerati.
echo Ricorda di committarli INSIEME a steam_messages.proto.
pause
