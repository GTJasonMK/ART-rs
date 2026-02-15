@echo off
setlocal EnableExtensions EnableDelayedExpansion
chcp 65001 >nul

set "SCRIPT_DIR=%~dp0"
if "%SCRIPT_DIR:~-1%"=="\" set "SCRIPT_DIR=%SCRIPT_DIR:~0,-1%"
cd /d "%SCRIPT_DIR%"

set "LOCK_FILE=%SCRIPT_DIR%\.art-rs-start.lock"
set "STATE_FILE=%SCRIPT_DIR%\.art-rs-start.state"
set "GEN_CONFIG=%SCRIPT_DIR%\.tauri.dev.generated.json"
set "GEN_PS=%SCRIPT_DIR%\.tauri.dev.generate.ps1"
set "APP_EXIT=0"

echo [INFO] Pre-start cleanup...
if exist "%LOCK_FILE%" del /f /q "%LOCK_FILE%" >nul 2>nul
if exist "%STATE_FILE%" del /f /q "%STATE_FILE%" >nul 2>nul
if exist "%GEN_CONFIG%" del /f /q "%GEN_CONFIG%" >nul 2>nul
if exist "%GEN_PS%" del /f /q "%GEN_PS%" >nul 2>nul
for %%F in ("%SCRIPT_DIR%\*.tmp" "%SCRIPT_DIR%\*.temp") do (
    if exist "%%~fF" del /f /q "%%~fF" >nul 2>nul
)

set "APP_PORT="
for /f %%P in ('powershell -NoProfile -Command "$listener=[System.Net.Sockets.TcpListener]::new([System.Net.IPAddress]::Loopback,0);$listener.Start();$p=$listener.LocalEndpoint.Port;$listener.Stop();Write-Output $p"') do (
    set "APP_PORT=%%P"
)
if not defined APP_PORT (
    echo [ERROR] Unable to acquire a free port.
    set "APP_EXIT=1"
)

if "%APP_EXIT%"=="0" (
    echo [INFO] Selected free port: %APP_PORT%
    set "PORT=%APP_PORT%"
    set "VITE_DEV_SERVER_PORT=%APP_PORT%"
    set "ART_RS_CONFIG_DIR=%SCRIPT_DIR%"

    > "%LOCK_FILE%" echo started_at=%DATE% %TIME%
    >> "%LOCK_FILE%" echo port=%APP_PORT%
    >> "%LOCK_FILE%" echo pid=%RANDOM%

    set "MODE=%~1"
    if not defined MODE set "MODE=dev"

    echo mode=%MODE%>"%STATE_FILE%"
    echo port=%APP_PORT%>>"%STATE_FILE%"
)

if "%APP_EXIT%"=="0" (
    if not exist "%SCRIPT_DIR%\node_modules" (
        echo [INFO] node_modules missing, running npm install...
        npm install
        if errorlevel 1 (
            echo [ERROR] npm install failed.
            set "APP_EXIT=1"
        )
    )
)

if "%APP_EXIT%"=="0" (
    if not exist "%SCRIPT_DIR%\src-tauri\tauri.conf.json" (
        echo [ERROR] Missing src-tauri\tauri.conf.json
        set "APP_EXIT=1"
    )
)

if "%APP_EXIT%"=="0" (
    copy /y "%SCRIPT_DIR%\src-tauri\tauri.conf.json" "%GEN_CONFIG%" >nul
    if errorlevel 1 (
        echo [ERROR] Copy base tauri config failed.
        set "APP_EXIT=1"
    )
)

if "%APP_EXIT%"=="0" (
    > "%GEN_PS%" (
        echo $ErrorActionPreference = 'Stop'
        echo $cfgPath = '%GEN_CONFIG%'
        echo if ^(-not ^(Test-Path -LiteralPath $cfgPath^)^) { throw 'generated config path missing' }
        echo $cfg = Get-Content -Raw -LiteralPath $cfgPath ^| ConvertFrom-Json
        echo if ^(-not $cfg.build^) { $cfg ^| Add-Member -MemberType NoteProperty -Name build -Value ^(@{}^) }
        echo $cfg.build.devUrl = 'http://127.0.0.1:%APP_PORT%'
        echo $cfg.build.beforeDevCommand = 'npm run dev -- --host 127.0.0.1 --port %APP_PORT% --strictPort'
        echo $json = $cfg ^| ConvertTo-Json -Depth 100
        echo Set-Content -LiteralPath $cfgPath -Encoding UTF8 -Value $json
        echo if ^(-not ^(Test-Path -LiteralPath $cfgPath^)^) { throw 'generated config not written' }
    )
    powershell -NoProfile -ExecutionPolicy Bypass -File "%GEN_PS%"
    if errorlevel 1 (
        echo [ERROR] Generate dynamic tauri config failed via PowerShell.
        set "APP_EXIT=1"
    )
    if exist "%GEN_PS%" del /f /q "%GEN_PS%" >nul 2>nul
)

if "%APP_EXIT%"=="0" (
    if not exist "%GEN_CONFIG%" (
        echo [ERROR] Generated config file missing: %GEN_CONFIG%
        set "APP_EXIT=1"
    )
)

if "%APP_EXIT%"=="0" (
    if /I "%MODE%"=="dev" (
        echo [INFO] Start mode: tauri dev
        npm run tauri:dev -- --config "%GEN_CONFIG%"
        set "APP_EXIT=%ERRORLEVEL%"
    ) else if /I "%MODE%"=="build" (
        echo [INFO] Start mode: tauri build
        npm run tauri:build -- --config "%GEN_CONFIG%"
        set "APP_EXIT=%ERRORLEVEL%"
    ) else (
        echo [ERROR] Unsupported mode: %MODE%
        echo [INFO] Supported modes: dev ^| build
        set "APP_EXIT=2"
    )
)

echo [INFO] Post-exit cleanup...
if exist "%LOCK_FILE%" del /f /q "%LOCK_FILE%" >nul 2>nul
if exist "%STATE_FILE%" del /f /q "%STATE_FILE%" >nul 2>nul
if exist "%GEN_CONFIG%" del /f /q "%GEN_CONFIG%" >nul 2>nul
if exist "%GEN_PS%" del /f /q "%GEN_PS%" >nul 2>nul
for %%F in ("%SCRIPT_DIR%\*.tmp" "%SCRIPT_DIR%\*.temp") do (
    if exist "%%~fF" del /f /q "%%~fF" >nul 2>nul
)

if "%APP_EXIT%"=="0" (
    echo [INFO] Finished successfully.
) else (
    echo [ERROR] Startup failed. Exit code: %APP_EXIT%
)
exit /b %APP_EXIT%
