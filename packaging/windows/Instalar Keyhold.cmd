@echo off
rem Instala o Keyhold a partir desta pasta (o programa ja vem pronto) e abre o Keyhold.
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0install.ps1"
if errorlevel 1 (
  echo.
  echo A instalacao nao terminou. Veja a mensagem acima.
  pause
  exit /b 1
)
start "" "%APPDATA%\Microsoft\Windows\Start Menu\Programs\Keyhold.lnk"
