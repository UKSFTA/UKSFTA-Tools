#!/usr/bin/env pwsh
# Install uksfta from the latest GitHub release.
# Usage: irm https://github.com/UKSFTA/UKSFTA-Tools/releases/latest/download/install.ps1 | iex
# The script downloads uksfta.exe, verifies it against the release's SHA256
# checksum or, when cosign is installed, its sigstore bundle, and adds it to
# the user PATH.

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

Write-Output "Downloading $Asset..."
$tmp = [System.IO.Path]::GetTempFileName()
Invoke-WebRequest -Uri $asset.browser_download_url -OutFile $tmp -UseBasicParsing

$verified = $false
$cosign = Get-Command cosign -ErrorAction SilentlyContinue
if ($cosign) {
    Write-Output "Verifying sigstore bundle..."
    $bundle = [System.IO.Path]::GetTempFileName()
    $bundleUrl = "https://github.com/$Repo/releases/download/$Tag/$Asset.sigstore.json"
    try {
        Invoke-WebRequest -Uri $bundleUrl -OutFile $bundle -UseBasicParsing
    } catch {
        Write-Output "Could not download sigstore bundle; falling back to checksum."
        Remove-Item $bundle -Force -ErrorAction SilentlyContinue
        $bundle = $null
    }
    if ($bundle) {
        & cosign verify-blob `
            --bundle $bundle `
            --certificate-identity-regexp "https://github.com/$Repo/.github/workflows/release.yml@refs/tags/.*" `
            --certificate-oidc-issuer "https://token.actions.githubusercontent.com" `
            $tmp
        if ($LASTEXITCODE -ne 0) {
            Remove-Item $tmp -Force
            Remove-Item $bundle -Force
            throw "Signature verification failed. Aborting."
        }
        $verified = $true
        Remove-Item $bundle -Force
        Write-Output "Signature verified."
    }
} else {
    Write-Output "cosign not found; skipping signature verification."
}

if (-not $verified) {
    $expected = Get-Checksum -Tag $tag -AssetName $Asset
    $actual = (Get-FileHash -Path $tmp -Algorithm SHA256).Hash.ToLower()
    if ($actual -ne $expected) {
        Remove-Item $tmp -Force
        throw "Checksum mismatch. Expected $expected, got $actual. Aborting."
    }
    Write-Output "Checksum verified."
}

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