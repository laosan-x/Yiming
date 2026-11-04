# Build the NSIS installer locally.
#
# The version is always read from Cargo.toml and passed to makensis, so it
# cannot drift from installer.nsi (which only keeps a "0.0.0-dev" sentinel
# default and emits a !warning when built bare).
#
# NOTE: this file is intentionally ASCII-only. Windows PowerShell 5.1 reads
# BOM-less UTF-8 as the system ANSI codepage, and any non-ASCII text here
# corrupts string literals at parse time. Keep all messages in English.
#
# Usage (from repo root):
#   pwsh -File scripts/build-installer.ps1
#   pwsh -File scripts/build-installer.ps1 -DryRun   # print plan only
param([switch]$DryRun)

$ErrorActionPreference = 'Stop'

$root = Split-Path -Parent $PSScriptRoot
$cargo = Join-Path $root 'Cargo.toml'
$match = Select-String -Path $cargo -Pattern '^version\s*=\s*"([^"]+)"' | Select-Object -First 1
if (-not $match) { throw "No version field found in $cargo" }
$version = $match.Matches[0].Groups[1].Value
Write-Host "APP_VERSION = $version (from Cargo.toml)"

$makensis = Join-Path $root 'nsis-3.10\Bin\makensis.exe'
if (-not (Test-Path $makensis)) {
    $makensis = Join-Path ${env:ProgramFiles(x86)} 'NSIS\makensis.exe'
}
if (-not (Test-Path $makensis)) {
    throw 'makensis.exe not found: install NSIS or place nsis-3.10/ in the repo root'
}

if ($DryRun) {
    Write-Host "[DryRun] command : makensis -DAPP_VERSION=$version installer.nsi"
    Write-Host "[DryRun] makensis: $makensis"
    Write-Host "[DryRun] output  : dist/cocole-iced-v$version-setup.exe"
    exit 0
}

Push-Location $root
try {
    & $makensis "-DAPP_VERSION=$version" 'installer.nsi'
    if ($LASTEXITCODE -ne 0) { throw "makensis failed with exit code $LASTEXITCODE" }
} finally {
    Pop-Location
}
Write-Host "Output: dist/cocole-iced-v$version-setup.exe"
