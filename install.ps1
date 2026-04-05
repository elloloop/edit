# edit — TUI code editor installer for Windows
# Usage: irm https://raw.githubusercontent.com/elloloop/edit/main/install.ps1 | iex

$ErrorActionPreference = "Stop"

$repo = "elloloop/edit"
$target = "x86_64-pc-windows-msvc"
$installDir = "$env:LOCALAPPDATA\edit"

Write-Host ""
Write-Host "  > edit" -ForegroundColor White -NoNewline
Write-Host " installer"
Write-Host "  A lightweight code viewer for AI agent workflows"
Write-Host ""

# Get latest release
Write-Host "  info  Fetching latest release..." -ForegroundColor Cyan
$release = Invoke-RestMethod "https://api.github.com/repos/$repo/releases/latest"
$tag = $release.tag_name
Write-Host "    ok  Latest release: $tag" -ForegroundColor Green

# Download TUI
$tuiAsset = $release.assets | Where-Object { $_.name -eq "edit-$target.exe" }
$guiAsset = $release.assets | Where-Object { $_.name -eq "edit-gui-$target.exe" }

if (-not (Test-Path $installDir)) {
    New-Item -ItemType Directory -Path $installDir -Force | Out-Null
}

if ($tuiAsset) {
    Write-Host "  info  Downloading edit.exe..." -ForegroundColor Cyan
    Invoke-WebRequest -Uri $tuiAsset.browser_download_url -OutFile "$installDir\edit.exe"
    Write-Host "    ok  Downloaded edit.exe" -ForegroundColor Green
}

if ($guiAsset) {
    Write-Host "  info  Downloading edit-gui.exe..." -ForegroundColor Cyan
    Invoke-WebRequest -Uri $guiAsset.browser_download_url -OutFile "$installDir\edit-gui.exe"
    Write-Host "    ok  Downloaded edit-gui.exe" -ForegroundColor Green
}

# Add to PATH
$currentPath = [Environment]::GetEnvironmentVariable("PATH", "User")
if ($currentPath -notlike "*$installDir*") {
    [Environment]::SetEnvironmentVariable("PATH", "$currentPath;$installDir", "User")
    Write-Host "    ok  Added $installDir to PATH" -ForegroundColor Green
}

# Set EDITOR
[Environment]::SetEnvironmentVariable("EDITOR", "edit", "User")
Write-Host "    ok  Set EDITOR=edit" -ForegroundColor Green

Write-Host ""
Write-Host "  Installation complete!" -ForegroundColor Green
Write-Host ""
Write-Host "  Restart your terminal, then run:"
Write-Host "    edit          — open current directory"
Write-Host "    edit file.rs  — open a file"
Write-Host "    edit-gui      — launch desktop app"
Write-Host ""
