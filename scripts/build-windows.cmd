@echo off
REM Double-click friendly wrapper for Windows 11 packaging
cd /d "%~dp0\.."
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0build-windows.ps1" %*
if errorlevel 1 pause
