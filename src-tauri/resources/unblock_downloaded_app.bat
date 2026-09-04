@echo off
setlocal
powershell.exe -NoLogo -NoProfile -NonInteractive -Command "$ErrorActionPreference='Stop'; Unblock-File -LiteralPath $env:PHONE_AUDIO_RECEIVER_EXE"
exit /b %ERRORLEVEL%
