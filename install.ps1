# Installs Model Viewer for the current user (no admin needed):
#   1. copies the built ModelViewer.exe and ModelViewerThumbnail.dll to
#      %LOCALAPPDATA%\Programs\ModelViewer
#   2. adds the viewer to the "Open with" list for .fbx and .obj files
#   3. registers the DLL as the Explorer thumbnail provider for .fbx and .obj
# Run again after every 'npm run build' to update the installed copy.
#   .\install.ps1              install / update
#   .\install.ps1 -Uninstall   remove the app, its "Open with" entry, the
#                              thumbnail provider and the thumbnail cache
param([switch]$Uninstall)

# Your own build (Cargo builds into src-tauri\target unless CARGO_TARGET_DIR
# points elsewhere) wins; without one, the prebuilt binaries in dist\ are used.
$targetDir = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $PSScriptRoot 'src-tauri\target' }
$sourceDir = if (Test-Path (Join-Path $targetDir 'release\ModelViewer.exe')) { Join-Path $targetDir 'release' } else { Join-Path $PSScriptRoot 'dist' }
$source    = Join-Path $sourceDir 'ModelViewer.exe'
$sourceDll = Join-Path $sourceDir 'ModelViewerThumbnail.dll'
$target = Join-Path $env:LOCALAPPDATA 'Programs\ModelViewer'
$exe    = Join-Path $target 'ModelViewer.exe'
$dll    = Join-Path $target 'ModelViewerThumbnail.dll'
$cache  = Join-Path $env:LOCALAPPDATA 'ModelViewer'
$key    = 'HKCU:\Software\Classes\Applications\ModelViewer.exe'
# CLSID of the thumbnail provider (src-tauri\thumbnail\src\lib.rs) and the
# shell's thumbnail-handler category it is registered under.
$clsid    = '{9C1F2E7A-5B3D-4E86-A0F4-6D2B8C1E7F35}'
$clsidKey = "HKCU:\Software\Classes\CLSID\$clsid"
$handler  = '{E357FCCD-A995-4576-B01F-234630154E96}'
$extensions = '.fbx', '.obj'

if (Get-Process ModelViewer -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $exe }) {
    Write-Error 'Model Viewer is running. Close all its windows and run this again.'
    return
}

# Tell Explorer that file associations changed, so it picks the provider up
# (or drops it) without a restart.
Add-Type -Namespace ModelViewer -Name Shell -MemberDefinition @'
[System.Runtime.InteropServices.DllImport("shell32.dll")]
public static extern void SHChangeNotify(int eventId, uint flags, System.IntPtr item1, System.IntPtr item2);
'@
function Update-Shell { [ModelViewer.Shell]::SHChangeNotify(0x08000000, 0, [IntPtr]::Zero, [IntPtr]::Zero) }

# Explorer keeps the DLL loaded for a while after showing a thumbnail. A loaded
# DLL cannot be deleted, but it can be renamed: park it as *.old and delete the
# parked copies that are free by now.
function Remove-InstalledDll {
    Get-ChildItem "$target\*.old" -ErrorAction SilentlyContinue | ForEach-Object {
        Remove-Item $_.FullName -Force -ErrorAction SilentlyContinue
    }
    if (-not (Test-Path $dll)) { return $true }
    try { Remove-Item $dll -Force -ErrorAction Stop; return $true } catch {}
    Rename-Item $dll ("ModelViewerThumbnail.{0}.old" -f (Get-Date).Ticks)
    return $false
}

if ($Uninstall) {
    foreach ($ext in $extensions) {
        $k = "HKCU:\Software\Classes\$ext\ShellEx\$handler"
        if (Test-Path $k) { Remove-Item $k -Recurse -Confirm:$false }
    }
    if (Test-Path $clsidKey) { Remove-Item $clsidKey -Recurse -Confirm:$false }
    if (Test-Path $key)      { Remove-Item $key -Recurse -Confirm:$false }
    $freed = Remove-InstalledDll
    if (Test-Path $target) { Remove-Item $target -Recurse -Force -Confirm:$false -ErrorAction SilentlyContinue }
    if (Test-Path $cache)  { Remove-Item $cache -Recurse -Force -Confirm:$false }
    Update-Shell
    Write-Host 'Model Viewer removed.'
    if (-not $freed) { Write-Host "Explorer still holds the thumbnail DLL; delete $target after the next sign-in." }
    return
}

foreach ($file in $source, $sourceDll) {
    if (-not (Test-Path $file)) {
        Write-Error "Not found: $file -- run 'npm run build', or check out dist\ from the repo."
        return
    }
}

# Start from a clean folder, so files of an older version do not linger.
$freed = Remove-InstalledDll
if (Test-Path $exe) { Remove-Item $exe -Force }
New-Item $target -ItemType Directory -Force | Out-Null
Copy-Item $source $exe
Copy-Item $sourceDll $dll

New-Item "$key\shell\open\command" -Force | Out-Null
New-Item "$key\SupportedTypes" -Force | Out-Null
Set-ItemProperty $key -Name 'FriendlyAppName' -Value 'Model Viewer'
Set-ItemProperty "$key\shell\open\command" -Name '(default)' -Value "`"$exe`" `"%1`""
foreach ($ext in $extensions) { Set-ItemProperty "$key\SupportedTypes" -Name $ext -Value '' }

# The provider runs inside Explorer: the shell's isolated thumbnail host
# (dllhost.exe) cannot create classes registered for the current user only.
New-Item "$clsidKey\InprocServer32" -Force | Out-Null
Set-ItemProperty $clsidKey -Name '(default)' -Value 'Model Viewer Thumbnail Provider'
Set-ItemProperty $clsidKey -Name 'DisableProcessIsolation' -Value 1 -Type DWord
Set-ItemProperty "$clsidKey\InprocServer32" -Name '(default)' -Value $dll
Set-ItemProperty "$clsidKey\InprocServer32" -Name 'ThreadingModel' -Value 'Apartment'
foreach ($ext in $extensions) {
    $k = "HKCU:\Software\Classes\$ext\ShellEx\$handler"
    New-Item $k -Force | Out-Null
    Set-ItemProperty $k -Name '(default)' -Value $clsid
}
Update-Shell

Write-Host "Installed $sourceDir to $target"
Write-Host 'Right-click an .fbx or .obj file -> Open with -> Choose another app -> Model Viewer -> Always.'
Write-Host 'Explorer shows a thumbnail for every model once it has been opened in Model Viewer.'
if (-not $freed) { Write-Host 'Explorer still uses the previous thumbnail DLL; it switches once it lets go of it, or after a restart.' }
