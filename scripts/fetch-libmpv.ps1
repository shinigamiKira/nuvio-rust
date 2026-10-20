<#
.SYNOPSIS
  Downloads the pinned libmpv Windows build and stages libmpv-2.dll under libs/windows-x64.
.NOTES
  The archive is a 7z, so 7-Zip is required (winget install 7zip.7zip).
  The build is GPL, which matches Nuvio's GPL-3.0 license. It is the same build the Kotlin port ships.
#>
[CmdletBinding()]
param(
    [string]$Url = "https://github.com/zhongfly/mpv-winbuild/releases/download/2026-10-19-6a262d11f6/mpv-dev-x86_64-20261019-git-6a262d11f6.7z",
    [string]$Sha256 = "2c71ab5cc5efd4bb055c6d21d7978ac0af9cad313c7673ab8acb362382d27dd1"
)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
$dllDir = Join-Path $root "libs\windows-x64"
$dll = Join-Path $dllDir "libmpv-2.dll"
if (Test-Path $dll) { Write-Output "libmpv already staged: $dll"; return }

$sevenZip = @("C:\Program Files\7-Zip\7z.exe", "C:\Program Files (x86)\7-Zip\7z.exe") | Where-Object { Test-Path $_ } | Select-Object -First 1
if (-not $sevenZip) { throw "7-Zip not found. Install it with: winget install 7zip.7zip" }

$work = Join-Path ([System.IO.Path]::GetTempPath()) ("nuvio-libmpv-" + [guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Path $work | Out-Null
try {
    $archive = Join-Path $work "mpv-dev.7z"
    Write-Output "Downloading $Url"
    & curl.exe -L --fail --silent --show-error -o $archive $Url
    if ($LASTEXITCODE -ne 0) { throw "download failed" }
    $actual = (Get-FileHash $archive -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actual -ne $Sha256.ToLowerInvariant()) { throw "SHA-256 mismatch: expected $Sha256, got $actual" }
    $extract = Join-Path $work "x"
    & $sevenZip x -y "-o$extract" $archive | Out-Null
    New-Item -ItemType Directory -Force -Path $dllDir | Out-Null
    Copy-Item (Join-Path $extract "libmpv-2.dll") $dllDir -Force
    Write-Output "Staged $dll"
}
finally {
    Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue
}
