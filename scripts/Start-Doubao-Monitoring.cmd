@echo off
echo Experimental Doubao desktop monitoring.
echo This enables debugging on 127.0.0.1:19224. Other local apps may access the page.
echo Exit Doubao first. Launch Doubao normally after exiting to disable debugging.
choice /C YN /N /M "Enable for this launch? [Y/N] "
if errorlevel 2 exit /b 0
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0Start-Doubao-Monitoring.ps1"
pause
