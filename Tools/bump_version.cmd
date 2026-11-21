@echo off
setlocal EnableExtensions
title Aether version bump
cd /d "%~dp0.."

:menu
echo.
echo   Aether version bump
echo   -------------------
echo   1^) AetherDesk
echo   2^) AetherDLL
echo   0^) Exit
echo.
set "CHOICE="
set /p "CHOICE=Choice: "
if errorlevel 1 goto :eof

if "%CHOICE%"=="0" goto :eof
if "%CHOICE%"=="1" (
  set "COMPONENT=Desk"
  set "LABEL=AetherDesk"
) else if "%CHOICE%"=="2" (
  set "COMPONENT=Dll"
  set "LABEL=AetherDLL"
) else (
  echo Invalid choice.
  goto :menu
)

set "VERSION="
set /p "VERSION=New %LABEL% version: "
if errorlevel 1 goto :eof

powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0bump_version.ps1" -Component %COMPONENT% -Version "%VERSION%"
if errorlevel 1 (
  echo.
  echo Version bump failed.
  pause
)
goto :menu
