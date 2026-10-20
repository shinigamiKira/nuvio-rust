<#
.SYNOPSIS
  Builds the release exe and wraps it, with libmpv, into dist\Nuvio-Rust-Setup-<version>.exe.
.NOTES
  Needs the Rust toolchain on PATH, Inno Setup 6 (winget install JRSoftware.InnoSetup) and libmpv in libs\windows-x64
  (scripts\fetch-libmpv.ps1 puts it there).
#>
[CmdletBinding()]
param(
    [switch]$SkipBuild
)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

$version = (Select-String -Path "Cargo.toml" -Pattern '^version\s*=\s*"([^"]+)"' | Select-Object -First 1).Matches.Groups[1].Value
if (-not $version) { throw "could not read the version from Cargo.toml" }

& "$PSScriptRoot\fetch-libmpv.ps1"

if (-not $SkipBuild) {
    cargo build -p nuvio --release
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }
}

# cargo may write to a target dir outside the repo (CARGO_TARGET_DIR or ~/.cargo/config.toml), so ask it.
$targetDir = (cargo metadata --format-version 1 --no-deps | ConvertFrom-Json).target_directory
$exe = Join-Path $targetDir "release\nuvio.exe"
if (-not (Test-Path $exe)) { throw "release build not found: $exe" }

$stage = Join-Path $root "dist\stage"
Remove-Item -Recurse -Force $stage -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force $stage | Out-Null
Copy-Item $exe $stage
Copy-Item (Join-Path $root "libs\windows-x64\libmpv-2.dll") $stage

$iscc = @(
    "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe",
    "C:\Program Files (x86)\Inno Setup 6\ISCC.exe",
    "C:\Program Files\Inno Setup 6\ISCC.exe"
) | Where-Object { Test-Path $_ } | Select-Object -First 1
if (-not $iscc) { throw "Inno Setup 6 not found. Install it with: winget install JRSoftware.InnoSetup" }

& $iscc "/DAppVersion=$version" "/DSourceDir=$stage" "/DOutputDir=$(Join-Path $root 'dist')" (Join-Path $root "installer\nuvio.iss")
if ($LASTEXITCODE -ne 0) { throw "Inno Setup failed" }

Remove-Item -Recurse -Force $stage
$setup = Join-Path $root "dist\Nuvio-Rust-Setup-$version.exe"
$hash = (Get-FileHash $setup -Algorithm SHA256).Hash.ToLowerInvariant()
Write-Output ("{0}  {1:N0} bytes  sha256 {2}" -f $setup, (Get-Item $setup).Length, $hash)
