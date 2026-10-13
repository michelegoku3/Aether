@echo off
setlocal EnableExtensions
REM ============================================================
REM regen_proto.cmd - Regenerate AetherDLL/proto/generated/steam_messages.pb.{h,cc}
REM
REM When needed: ONLY after editing AetherDLL/proto/steam_messages.proto
REM (with AETHER_PROTO_PREGEN=ON, default, the .pb files are pre-generated and
REM  committed in the repo: protoc is not even built).
REM
REM Requirement: protoc MUST be version 25.3 (same as the protobuf v25.3
REM runtime used by FetchContent).
REM ============================================================

set "ROOT=%~dp0.."
set "PROTO_DIR=%ROOT%\AetherDLL\proto"
set "OUT_DIR=%PROTO_DIR%\generated"
set "PROTOC=%ROOT%\AetherDLL\out\build\x64-Release\_deps\protobuf-build\protoc.exe"

if exist "%PROTOC%" goto :run

REM Fallback: a protoc in PATH (make sure it is 25.3 yourself)
where protoc >nul 2>nul
if %errorlevel%==0 (
    echo [WARNING] Using the protoc found in PATH: it MUST be version 25.3.
    protoc --version
    set "PROTOC=protoc"
    goto :run
)

echo [ERROR] protoc.exe not found.
echo.
echo  With AETHER_PROTO_PREGEN=ON protoc is not built. Alternatives:
echo   1. configure a build with -DAETHER_PROTO_PREGEN=OFF, build the protoc
echo      target once, then re-run this script;
echo   2. or download protoc 25.3 from:
echo      https://github.com/protocolbuffers/protobuf/releases/tag/v25.3
echo      ^(protoc-25.3-win64.zip^), put it in PATH and re-run.
echo.
pause
exit /b 1

:run
if not exist "%OUT_DIR%" mkdir "%OUT_DIR%"
"%PROTOC%" --cpp_out=lite:"%OUT_DIR%" -I "%PROTO_DIR%" "%PROTO_DIR%\steam_messages.proto"
if errorlevel 1 (
    echo [ERROR] protoc failed.
    pause
    exit /b 1
)
echo.
echo Done: %OUT_DIR%\steam_messages.pb.h and .cc regenerated.
echo Remember to commit them TOGETHER with steam_messages.proto.
pause
