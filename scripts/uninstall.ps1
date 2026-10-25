# Remove MICA installed by scripts\install.ps1: runs `mica uninstall` (which
# asks before deleting MICA's data), deletes mica.exe, and removes its folder
# from your user PATH.

$ErrorActionPreference = 'Stop'
$InstallDir = if ($env:MICA_INSTALL_DIR) { $env:MICA_INSTALL_DIR } else { Join-Path $env:LOCALAPPDATA 'Programs\MICA' }
$mica = Join-Path $InstallDir 'mica.exe'

if (Test-Path -LiteralPath $mica) {
    & $mica uninstall
    Remove-Item -LiteralPath $mica -Force
    if (-not (Get-ChildItem -LiteralPath $InstallDir -Force)) { Remove-Item -LiteralPath $InstallDir -Force }
    Write-Host "mica-uninstall: removed $mica"
} else {
    Write-Host "mica-uninstall: mica.exe is not installed in $InstallDir"
}
if ($env:MICA_NO_PATH -eq '1') { return }
$userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
$parts = @($userPath -split ';' | Where-Object { $_ -and $_ -ne $InstallDir })
if ($parts.Count -ne @($userPath -split ';' | Where-Object { $_ }).Count) {
    [Environment]::SetEnvironmentVariable('Path', ($parts -join ';'), 'User')
    Write-Host "mica-uninstall: removed $InstallDir from your user PATH"
}
