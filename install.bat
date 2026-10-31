@echo off
setlocal

set "INSTALL_DIR=%LOCALAPPDATA%\StrawberrySocial"
set "BIN_NAME=strawberry-social.exe"

echo Installing Strawberry Social...

:: Create necessary local directories
if not exist "%INSTALL_DIR%\characters" mkdir "%INSTALL_DIR%\characters"
if not exist "%INSTALL_DIR%\sessions" mkdir "%INSTALL_DIR%\sessions"

:: Locate and copy binary
if exist "target\release\%BIN_NAME%" (
    copy /y "target\release\%BIN_NAME%" "%INSTALL_DIR%\" >nul
) else if exist "%BIN_NAME%" (
    copy /y "%BIN_NAME%" "%INSTALL_DIR%\" >nul
) else (
    echo Error: Binary %BIN_NAME% not found. Build it first!
    exit /b 1
)

:: Copy config if not already present
if exist "config.yaml" if not exist "%INSTALL_DIR%\config.yaml" (
    copy /y "config.yaml" "%INSTALL_DIR%\" >nul
)

:: Create desktop shortcut using a lightweight shell object call
echo Creating desktop shortcut...
powershell -NoProfile -Command "$ws = New-Object -ComObject WScript.Shell; $sc = $ws.CreateShortcut([Environment]::GetFolderPath('Desktop') + '\Strawberry Social.lnk'); $sc.TargetPath = '%INSTALL_DIR%\%BIN_NAME%'; $sc.WorkingDirectory = '%INSTALL_DIR%'; $sc.Description = 'Strawberry Social AI Chat'; $sc.Save()" >nul

echo.
echo Installation complete.
echo Installed to: %INSTALL_DIR%
echo A shortcut has been placed on your Desktop.
pause