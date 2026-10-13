@echo off
REM Double click: sync AetherDLL source timestamps after replacing downloaded
REM files, then use "Build All" in Visual Studio.
REM -Root is passed explicitly: root = parent folder of Tools\.
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0sync_aetherdll.ps1" -Root "%~dp0.."
echo.
pause
