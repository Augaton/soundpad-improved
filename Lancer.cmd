@echo off
rem Lance Soundpad Improved sans garder de fenêtre de console.
rem ELECTRON_RUN_AS_NODE (posé par certains éditeurs) ferait démarrer Electron en simple Node.
set ELECTRON_RUN_AS_NODE=
if not exist "%~dp0node_modules\electron\dist\electron.exe" (
  echo Dependances absentes : lance d'abord "npm install" dans ce dossier.
  pause
  exit /b 1
)
start "" "%~dp0node_modules\electron\dist\electron.exe" "%~dp0."
