#!/usr/bin/env pwsh
# Install uksfta from the latest GitHub release.
# Usage: irm https://github.com/UKSFTA/UKSFTA-Tools/releases/latest/download/install.ps1 | iex
# The script downloads uksfta.exe, verifies its SHA256 checksum, and adds it to the user PATH.

$ErrorActionPreference = "Stop"

$Repo = "UKSFTA/UKSFTA-Tools"
$Asset = "uksfta.exe"
$InstallDir = Join-Path $env:LOCALAPPDATA "Programs\uksfta"
$ExePath = Join-Path $InstallDir $Asset

function Get-LatestRelease {
    $release = Invoke-RestMethod -Uri "https://api.github.com/repos/$Repo/releases/latest" -Headers @{ "User-Agent" = "uksfta-installer" }
    return $release
}

function Get-Checksum {
    param([string]$Tag, [string]$AssetName)
    $sums = Invoke-WebRequest -Uri "https://github.com/$Repo/releases/download/$Tag/SHA256SUMS" -UseBasicParsing
    foreach ($line in ($sums.Content -split "`n")) {
        if ($line -match "([0-9a-f]{64})\s+\*?([^\s]+)") {
            if ($matches[2] -eq $AssetName) {
                return $matches[1]
            }
        }
    }
    throw "Checksum for $AssetName not found in SHA256SUMS"
}

Write-Output "Finding latest uksfta release..."
$release = Get-LatestRelease
$tag = $release.tag_name
Write-Output "Latest release: $tag"

# Locate the exe asset
$asset = $release.assets | Where-Object { $_.name -eq $Asset }
if (-not $asset) {
    throw "Asset $Asset not found in release $tag"
}

$expected = Get-Checksum -Tag $tag -AssetName $Asset

Write-Output "Downloading $Asset..."
$tmp = Join-Path $env:TEMP $Asset
Invoke-WebRequest -Uri $asset.browser_download_url -OutFile $tmp -UseBasicParsing

$actual = (Get-FileHash -Path $tmp -Algorithm SHA256).Hash.ToLower()
if ($actual -ne $expected) {
    Remove-Item $tmp -Force
    throw "Checksum mismatch. Expected $expected, got $actual. Aborting."
}
Write-Output "Checksum verified."

New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
Move-Item -Path $tmp -Destination $ExePath -Force

# Add to user PATH if not already present
$userPath = [Environment]::GetEnvironmentVariable("Path", "User")
if ($userPath -notlike "*$InstallDir*") {
    $newPath = if ($userPath) { "$userPath;$InstallDir" } else { $InstallDir }
    [Environment]::SetEnvironmentVariable("Path", $newPath, "User")
    Write-Output "Added $InstallDir to user PATH."
}

Write-Output "Installed uksfta $tag to $ExePath"
Write-Output "Open a new terminal and run 'uksfta --help' to verify."