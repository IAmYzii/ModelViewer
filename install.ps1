# Installs Model Viewer for the current user (no admin needed):
#   1. copies the built app to %LOCALAPPDATA%\Programs\ModelViewer
#   2. adds it to the "Open with" list for .fbx and .obj files
# Run again after every 'npm run build' to update the installed copy.
#   .\install.ps1              install / update
#   .\install.ps1 -Uninstall   remove the app and its "Open with" entry
param([switch]$Uninstall)

$source = Join-Path $PSScriptRoot 'dist\ModelViewer-win32-x64'
$target = Join-Path $env:LOCALAPPDATA 'Programs\ModelViewer'
$exe    = Join-Path $target 'ModelViewer.exe'
$key    = 'HKCU:\Software\Classes\Applications\ModelViewer.exe'

if (Get-Process ModelViewer -ErrorAction SilentlyContinue) {
    Write-Error 'Model Viewer is running. Close all its windows and run this again.'
    return
}

if ($Uninstall) {
    if (Test-Path $key)    { Remove-Item $key -Recurse -Confirm:$false }
    if (Test-Path $target) { Remove-Item $target -Recurse -Force -Confirm:$false }
    Write-Host 'Model Viewer removed.'
    return
}

if (-not (Test-Path (Join-Path $source 'ModelViewer.exe'))) {
    Write-Error "Build not found in $source -- run 'npm run build' first."
    return
}

if (Test-Path $target) { Remove-Item $target -Recurse -Force -Confirm:$false }
Copy-Item $source $target -Recurse

New-Item "$key\shell\open\command" -Force | Out-Null
New-Item "$key\SupportedTypes" -Force | Out-Null
Set-ItemProperty $key -Name 'FriendlyAppName' -Value 'Model Viewer'
Set-ItemProperty "$key\shell\open\command" -Name '(default)' -Value "`"$exe`" `"%1`""
Set-ItemProperty "$key\SupportedTypes" -Name '.fbx' -Value ''
Set-ItemProperty "$key\SupportedTypes" -Name '.obj' -Value ''

Write-Host "Installed to $target"
Write-Host 'Right-click an .fbx or .obj file -> Open with -> Choose another app -> Model Viewer -> Always.'
