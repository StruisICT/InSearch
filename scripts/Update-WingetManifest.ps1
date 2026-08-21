<#
.SYNOPSIS
    Generate the winget manifest folder for a released version of InSearch.

.DESCRIPTION
    Downloads the published InSearch MSI release asset, computes its SHA256 and
    reads its ProductCode, then writes the three-file winget manifest set under
    winget/manifests/s/StruisICT/InSearch/<Version>/ using schema 1.12.0.

    This only stages the manifest *in this repo*. It does NOT submit anything to
    microsoft/winget-pkgs — copying the folder into a winget-pkgs fork and
    opening that PR stays a deliberate, manual step (see winget/README.md).

.EXAMPLE
    pwsh ./scripts/Update-WingetManifest.ps1 -Version 0.2.0
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$Version,

    # Defaults to v<Version> — the tag release-please creates.
    [string]$Tag = "v$Version",

    # YYYY-MM-DD; defaults to today (UTC).
    [string]$ReleaseDate = ([DateTime]::UtcNow.ToString('yyyy-MM-dd')),

    # Optional release notes body. Review/refine in the PR before submitting.
    [string]$ReleaseNotes = ''
)

$ErrorActionPreference = 'Stop'

$repo     = 'StruisICT/InSearch'
$asset    = "InSearch-$Version-x86_64.msi"
$assetUrl = "https://github.com/$repo/releases/download/$Tag/$asset"
$notesUrl = "https://github.com/$repo/releases/tag/$Tag"

$repoRoot = Split-Path -Parent $PSScriptRoot
$outDir   = Join-Path $repoRoot "winget/manifests/s/StruisICT/InSearch/$Version"

Write-Host "Resolving release asset: $assetUrl"
$tmp = Join-Path ([System.IO.Path]::GetTempPath()) $asset
Invoke-WebRequest -Uri $assetUrl -OutFile $tmp -UseBasicParsing
# .NET SHA256 (portable across PowerShell versions; avoids Get-FileHash).
$sha256alg = [System.Security.Cryptography.SHA256]::Create()
$sha = ([System.BitConverter]::ToString($sha256alg.ComputeHash([System.IO.File]::ReadAllBytes($tmp))) -replace '-', '').ToUpperInvariant()
Write-Host "SHA256: $sha"

# Read the ProductCode out of the MSI's Property table (WindowsInstaller COM).
function Get-MsiProductCode([string]$Path) {
    $wi = New-Object -ComObject WindowsInstaller.Installer
    $db = $wi.GetType().InvokeMember('OpenDatabase', 'InvokeMethod', $null, $wi, @($Path, 0))
    $view = $db.GetType().InvokeMember('OpenView', 'InvokeMethod', $null, $db,
        @("SELECT Value FROM Property WHERE Property = 'ProductCode'"))
    $view.GetType().InvokeMember('Execute', 'InvokeMethod', $null, $view, $null) | Out-Null
    $rec = $view.GetType().InvokeMember('Fetch', 'InvokeMethod', $null, $view, $null)
    $code = $rec.GetType().InvokeMember('StringData', 'GetProperty', $null, $rec, @(1))
    [System.Runtime.InteropServices.Marshal]::ReleaseComObject($wi) | Out-Null
    return $code
}
$productCode = Get-MsiProductCode $tmp
Write-Host "ProductCode: $productCode"

if (-not $ReleaseNotes) {
    $ReleaseNotes = "See the full release notes at $notesUrl"
}
# Indent each release-notes line by two spaces for the YAML block scalar.
$notesBlock = ($ReleaseNotes -split "`r?`n" | ForEach-Object { "  $_" }) -join "`n"

New-Item -ItemType Directory -Force -Path $outDir | Out-Null

$installer = @"
# Created with: scripts/Update-WingetManifest.ps1
# yaml-language-server: `$schema=https://aka.ms/winget-manifest.installer.1.12.0.schema.json

PackageIdentifier: StruisICT.InSearch
PackageVersion: $Version
MinimumOSVersion: 10.0.0.0
InstallerType: msi
Scope: machine
InstallModes:
- interactive
- silent
- silentWithProgress
ReleaseDate: $ReleaseDate
Installers:
- Architecture: x64
  InstallerUrl: $assetUrl
  InstallerSha256: $sha
  ProductCode: '$productCode'
ManifestType: installer
ManifestVersion: 1.12.0
"@

$locale = @"
# Created with: scripts/Update-WingetManifest.ps1
# yaml-language-server: `$schema=https://aka.ms/winget-manifest.defaultLocale.1.12.0.schema.json

PackageIdentifier: StruisICT.InSearch
PackageVersion: $Version
PackageLocale: en-US
Publisher: Struis ICT
PublisherUrl: https://struisict.com
PublisherSupportUrl: https://github.com/StruisICT/InSearch/issues
PackageName: InSearch
PackageUrl: https://github.com/StruisICT/InSearch
License: MIT
LicenseUrl: https://github.com/StruisICT/InSearch/blob/main/LICENSE
Copyright: Copyright (c) 2026 Struis ICT
ShortDescription: Real-time, content-aware file search — find files and search inside them.
Description: |-
  InSearch is a real-time file search tool that finds files on disk and searches
  inside them, streaming matches as it goes. It searches plain text and logs plus
  PDF, Word (docx) and Excel (xls/xlsx/ods), with matches reported per line or per
  timestamp-to-timestamp block. Query with substring, regex, all-words (AND),
  any-words (OR) and exclude (NOT); filter by name, extension, size and date;
  watch folders for changes (log-tailing); export results; and search a folder
  straight from the Explorer right-click menu. Everything runs locally.
Moniker: insearch
Tags:
- search
- file-search
- content-search
- grep
- find
- logs
- utility
- windows
ReleaseNotes: |-
$notesBlock
ReleaseNotesUrl: $notesUrl
ManifestType: defaultLocale
ManifestVersion: 1.12.0
"@

$version = @"
# Created with: scripts/Update-WingetManifest.ps1
# yaml-language-server: `$schema=https://aka.ms/winget-manifest.version.1.12.0.schema.json

PackageIdentifier: StruisICT.InSearch
PackageVersion: $Version
DefaultLocale: en-US
ManifestType: version
ManifestVersion: 1.12.0
"@

# winget tooling expects UTF-8 (no BOM) with LF line endings.
$enc = New-Object System.Text.UTF8Encoding($false)
[System.IO.File]::WriteAllText((Join-Path $outDir 'StruisICT.InSearch.installer.yaml'),    ($installer -replace "`r`n","`n") + "`n", $enc)
[System.IO.File]::WriteAllText((Join-Path $outDir 'StruisICT.InSearch.locale.en-US.yaml'), ($locale    -replace "`r`n","`n") + "`n", $enc)
[System.IO.File]::WriteAllText((Join-Path $outDir 'StruisICT.InSearch.yaml'),              ($version    -replace "`r`n","`n") + "`n", $enc)

Write-Host ""
Write-Host "Wrote manifest set to: $outDir"
Get-ChildItem $outDir | ForEach-Object { Write-Host "  $($_.Name)" }
Write-Host ""
Write-Host "Next: review the ReleaseNotes/Description, then (when you choose to)"
Write-Host "copy this folder into a microsoft/winget-pkgs fork and open that PR."
