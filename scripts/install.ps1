# Install the `mica` command for the current user on Windows 10 or 11.
#
#     irm https://raw.githubusercontent.com/SwashX-dev/MICA-Multi-platform-Interoperability-Compatibility-Applicator-/main/scripts/install.ps1 | iex
#
# What it does, and nothing more:
#   1. picks the release build for this CPU (x86-64 or ARM64);
#   2. downloads it with the release's SHA256SUMS and refuses to install it
#      unless the checksum matches;
#   3. copies mica.exe into %LOCALAPPDATA%\Programs\MICA and adds that folder
#      to *your* PATH (not the system's). scripts\uninstall.ps1 undoes both.
# It needs no administrator rights. Before the first release, or with
# $env:MICA_FROM_SOURCE = "1", it builds MICA from source with Cargo.
#
# Settings: $env:MICA_VERSION (a tag, default: latest), $env:MICA_INSTALL_DIR,
# $env:MICA_BINARY (install this mica.exe), $env:MICA_RELEASE_BASE (testing),
# $env:MICA_NO_PATH = "1" (leave PATH alone).

$ErrorActionPreference = 'Stop'
$Repo = 'SwashX-dev/MICA-Multi-platform-Interoperability-Compatibility-Applicator-'
$InstallDir = if ($env:MICA_INSTALL_DIR) { $env:MICA_INSTALL_DIR } else { Join-Path $env:LOCALAPPDATA 'Programs\MICA' }

function Say($Message) { Write-Host "mica-install: $Message" }

function Install-Binary($File) {
    New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
    Copy-Item -LiteralPath $File -Destination (Join-Path $InstallDir 'mica.exe') -Force
    Say "installed $(Join-Path $InstallDir 'mica.exe')"
}

function Finish {
    & (Join-Path $InstallDir 'mica.exe') --version
    if ($LASTEXITCODE -ne 0) { throw 'the installed mica.exe does not run on this system' }
    $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
    $parts = @($userPath -split ';' | Where-Object { $_ })
    if ($env:MICA_NO_PATH -eq '1') {
        Say "left PATH unchanged (MICA_NO_PATH); run $(Join-Path $InstallDir 'mica.exe') directly"
    } elseif ($parts -notcontains $InstallDir) {
        [Environment]::SetEnvironmentVariable('Path', (($parts + $InstallDir) -join ';'), 'User')
        Say "added $InstallDir to your user PATH; open a new terminal to use it"
    }
    $env:Path = "$env:Path;$InstallDir"
    Say 'next: run `mica doctor` to see what this computer can run'
}

function From-Source {
    Say 'building MICA from source with Cargo'
    if (Get-Command rustup -ErrorAction SilentlyContinue) {
        rustup toolchain install 1.99.1 --profile minimal | Out-Null
        $cargo = @('rustup', 'run', '1.99.1', 'cargo')
    } elseif (Get-Command cargo -ErrorAction SilentlyContinue) {
        $cargo = @('cargo')
    } else {
        throw 'no release build yet, and Rust is not installed. Install it with `winget install Rustlang.Rustup` (it also needs the Visual Studio C++ Build Tools), then run this again'
    }
    $root = Split-Path $InstallDir -Parent
    $branch = if ($env:MICA_BRANCH) { $env:MICA_BRANCH } else { 'main' }
    $arguments = $cargo[1..($cargo.Count - 1)] + @('install', '--locked', '--git', "https://github.com/$Repo", '--branch', $branch, '--root', $root, 'mica-cli')
    & $cargo[0] @arguments
    if ($LASTEXITCODE -ne 0) { throw 'the source build failed (MICA needs Rust 1.99 or newer)' }
    Install-Binary (Join-Path $root 'bin\mica.exe')
    Finish
}

if ($env:MICA_BINARY) {
    Install-Binary $env:MICA_BINARY
    Finish
    return
}

$target = switch ($env:PROCESSOR_ARCHITECTURE) {
    'AMD64' { 'x86_64-pc-windows-msvc' }
    'ARM64' { 'aarch64-pc-windows-msvc' }
    default { $null }
}
if (-not $target -or $env:MICA_FROM_SOURCE -eq '1') { From-Source; return }

$version = $env:MICA_VERSION
if (-not $version -and -not $env:MICA_RELEASE_BASE) {
    try {
        $version = (Invoke-RestMethod "https://api.github.com/repos/$Repo/releases/latest").tag_name
    } catch {
        $version = $null
    }
}
if (-not $version -and -not $env:MICA_RELEASE_BASE) {
    Say 'no MICA release has been published yet'
    From-Source
    return
}

$base = if ($env:MICA_RELEASE_BASE) { $env:MICA_RELEASE_BASE } else { "https://github.com/$Repo/releases/download/$version" }
$asset = "mica-$target.zip"
$work = Join-Path ([IO.Path]::GetTempPath()) ("mica-install-" + [Guid]::NewGuid())
New-Item -ItemType Directory -Path $work | Out-Null
try {
    Say "downloading $asset $version"
    Invoke-WebRequest -UseBasicParsing -Uri "$base/$asset" -OutFile (Join-Path $work $asset)
    Invoke-WebRequest -UseBasicParsing -Uri "$base/SHA256SUMS" -OutFile (Join-Path $work 'SHA256SUMS')
    $line = Get-Content (Join-Path $work 'SHA256SUMS') | Where-Object { ($_ -split '\s+')[1] -in @($asset, "*$asset") } | Select-Object -First 1
    if (-not $line) { throw "SHA256SUMS does not list $asset; not installing an unverified file" }
    $expected = ($line -split '\s+')[0].ToLowerInvariant()
    $actual = (Get-FileHash -Algorithm SHA256 (Join-Path $work $asset)).Hash.ToLowerInvariant()
    if ($expected -ne $actual) { throw "checksum mismatch for $asset (expected $expected, got $actual); nothing was installed" }
    Say "checksum verified ($actual)"
    Expand-Archive -LiteralPath (Join-Path $work $asset) -DestinationPath $work -Force
    Install-Binary (Join-Path $work 'mica.exe')
    Finish
} finally {
    Remove-Item -Recurse -Force -LiteralPath $work -ErrorAction SilentlyContinue
}
