@echo off
chcp 65001 >nul 2>&1
setlocal enabledelayedexpansion

set "ROOT=%~dp0"
set "NAME=ART-rs"
set "VER=0.1.0"
set "OUT=%ROOT%dist-release\%NAME%-%VER%"

echo ============================================
echo  %NAME% v%VER% - Release Build
echo ============================================
echo.

echo [1/3] npm install ...
cd /d "%ROOT%"
call npm install
if errorlevel 1 (
    echo [ERROR] npm install failed
    exit /b 1
)

echo [2/3] tauri build ...
call npx tauri build
if errorlevel 1 (
    echo [ERROR] tauri build failed
    exit /b 1
)

echo [3/3] Packaging ...

if exist "%OUT%" rmdir /s /q "%OUT%"
mkdir "%OUT%"

copy /y "%ROOT%src-tauri\target\release\art_rs.exe" "%OUT%\%NAME%.exe" >nul
if exist "%ROOT%config.json" copy /y "%ROOT%config.json" "%OUT%\config.json" >nul
if not exist "%OUT%\credentials.txt" echo. > "%OUT%\credentials.txt"

set "NSIS_DIR=%ROOT%src-tauri\target\release\bundle\nsis"
if exist "%NSIS_DIR%" (
    for %%f in ("%NSIS_DIR%\*.exe") do (
        copy /y "%%f" "%OUT%\" >nul
        echo  Installer: %%~nxf
    )
)

for %%f in ("%OUT%\%NAME%.exe") do set "SIZE=%%~zf"
set /a "SIZE_MB=!SIZE! / 1048576"

echo.
echo ============================================
echo  Build OK
echo  Output: %OUT%
echo  Binary: %NAME%.exe (!SIZE_MB! MB)
echo ============================================
echo.

endlocal
