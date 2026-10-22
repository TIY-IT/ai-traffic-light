@echo off
echo Doubao signal check: samples for 75 seconds.
echo SEND ONE MESSAGE IN DOUBAO right after this window appears.
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0Check-Doubao-Signal.ps1"
echo.
echo Done. Send the Check-Doubao-result.txt file next to this window back to support.
pause
