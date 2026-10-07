@echo off
cd /d "%~dp0"
chcp 65001 >nul
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0pack.ps1"
set ERR=%ERRORLEVEL%
if not "%PACK_NO_PAUSE%"=="1" pause
exit /b %ERR%
