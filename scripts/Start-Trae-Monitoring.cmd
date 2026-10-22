@echo off
echo Experimental TRAE desktop monitoring.
echo This enables debugging on 127.0.0.1:19226. Other local apps may access the page.
echo Exit TRAE first. Launch TRAE normally after exiting to disable debugging.
choice /C YN /N /M "Enable for this launch? [Y/N] "
if errorlevel 2 exit /b 0
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0Start-Trae-Monitoring.ps1"
pause
