@echo off
setlocal
set "PHONE_AUDIO_RECEIVER_EXE=%~dp0Phone Audio Receiver.exe"
if not exist "%PHONE_AUDIO_RECEIVER_EXE%" exit /b 1
powershell.exe -NoLogo -NoProfile -NonInteractive -Command "$ErrorActionPreference='Stop'; Unblock-File -LiteralPath $env:PHONE_AUDIO_RECEIVER_EXE"
exit /b %ERRORLEVEL%
