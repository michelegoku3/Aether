@echo off
REM Doppio click: sincronizza i timestamp dei sorgenti AetherDLL dopo la
REM sostituzione dei file scaricati, poi usa "Compila tutto" in Visual Studio.
REM -Root viene passato esplicitamente: radice = cartella padre di Tools\.
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0sync_aetherdll.ps1" -Root "%~dp0.."
echo.
pause
