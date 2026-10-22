@echo off
echo Experimental Coze desktop monitoring.
echo This enables debugging on 127.0.0.1:19222. Other local apps may access the page.
echo Exit Coze first. Launch Coze normally after exiting to disable debugging.
choice /C YN /N /M "Enable for this launch? [Y/N] "
if errorlevel 2 exit /b 0
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0Start-Coze-Monitoring.ps1"
pause
