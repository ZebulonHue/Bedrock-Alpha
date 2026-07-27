@echo off
REM Project Bedrock — build if needed, then run.
REM
REM Double-click this. The first run compiles the app and takes a few minutes;
REM every run after that starts immediately. Shipping the built .exe in the
REM repository instead would add ~50 MB to every clone and go stale the moment
REM anyone changed a line, so it is built on your machine from the source.

setlocal
cd /d "%~dp0"

where cargo >nul 2>nul
if errorlevel 1 (
    echo.
    echo Rust is not installed, and it is needed to build the app.
    echo Install it from https://rustup.rs then double-click this file again.
    echo.
    pause
    exit /b 1
)

if not exist "target\release\project-bedrock.exe" (
    echo First run: building Project Bedrock. This takes a few minutes.
    echo.
)

cargo build --release
if errorlevel 1 (
    echo.
    echo The build failed. The errors above say why.
    echo.
    pause
    exit /b 1
)

start "" "target\release\project-bedrock.exe"
endlocal
