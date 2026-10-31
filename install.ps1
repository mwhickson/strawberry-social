$InstallDir = "$env:LOCALAPPDATA\StrawberrySocial"
$BinName = "strawberry-social.exe"

Write-Host "Installing Strawberry Social..."

# Create necessary local directories
New-Item -ItemType Directory -Force -Path "$InstallDir\characters" | Out-Null
New-Item -ItemType Directory -Force -Path "$InstallDir\sessions" | Out-Null

# Locate and copy binary
if (Test-Path "target\release\$BinName") {
    Copy-Item "target\release\$BinName" "$InstallDir\" -Force
} elseif (Test-Path "$BinName") {
    Copy-Item "$BinName" "$InstallDir\" -Force
} else {
    Write-Error "Binary $BinName not found. Build it first!"
    exit 1
}

# Copy config if not already present
if ((Test-Path "config.yaml") -and (-not (Test-Path "$InstallDir\config.yaml"))) {
    Copy-Item "config.yaml" "$InstallDir\"
}

# Create a Desktop shortcut with the correct working directory set
$WScriptShell = New-Object -ComObject WScript.Shell
$Shortcut = $WScriptShell.CreateShortcut("$env:USERPROFILE\Desktop\Strawberry Social.lnk")
$Shortcut.TargetPath = "$InstallDir\$BinName"
$Shortcut.WorkingDirectory = "$InstallDir"
$Shortcut.Description = "Strawberry Social AI Chat"
$Shortcut.Save()

Write-Host "Installation complete."
Write-Host "Installed to: $InstallDir"
Write-Host "A shortcut has been placed on your Desktop."