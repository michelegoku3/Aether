# ============================================================================
# sync_aetherdll.ps1 - Refresh AetherDLL source timestamps
# ----------------------------------------------------------------------------
# WHY IT EXISTS:
#   Files downloaded from GitHub / browser agents carry the commit time as
#   their "modified" date, often OLDER than the objects already compiled in
#   AetherDLL\out\. Windows copy-paste preserves that date: ninja then believes
#   nothing changed and "Build All" does not recompile the replaced files
#   (hence the need for "Rebuild All").
#
# WHAT IT DOES:
#   Compares each source file's content HASH against the last run and refreshes
#   the timestamp ONLY of files that really changed. After this script,
#   "Build > Build All" recompiles exactly the AI agent's modified files and
#   nothing else (protobuf/lua/minhook stay intact because they live in out\
#   and are untouched).
#
# USAGE:
#   1) Replace the files in AetherDLL (copy-paste, WITHOUT deleting the out\
#      folder).
#   2) Run this script (double click sync_aetherdll.cmd).
#   3) In Visual Studio: Build > Build All  (NOT "Rebuild All").
#
# TECHNICAL NOTE:
#   The repository root is NOT resolved in param() defaults: in Windows
#   PowerShell 5.1 $PSScriptRoot is an empty string there (it is populated only
#   once the script runs). Resolution happens in the script body with multiple
#   fallbacks, so it works from any launch mode (double click, "Run with
#   PowerShell", terminal, ISE).
# ============================================================================

[CmdletBinding()]
param(
    # Optional: repository root. When omitted, it is detected on its own.
    [string]$Root = ''
)

$ErrorActionPreference = 'Stop'

function Find-AetherRoot {
    param([string[]]$StartDirs)
    foreach ($dir in $StartDirs) {
        if ([string]::IsNullOrWhiteSpace($dir)) { continue }
        try { $dir = [IO.Path]::GetFullPath($dir) } catch { continue }
        # Walk up the tree (max 5 levels) looking for a folder holding AetherDLL
        $probe = $dir
        for ($i = 0; $i -lt 5; $i++) {
            if (Test-Path (Join-Path $probe 'AetherDLL')) { return $probe }
            $parent = $null
            try { $parent = [IO.Path]::GetDirectoryName($probe) } catch { $parent = $null }
            if ([string]::IsNullOrEmpty($parent) -or $parent -eq $probe) { break }
            $probe = $parent
        }
    }
    return $null
}

if ([string]::IsNullOrWhiteSpace($Root)) {
    $startDirs = @()
    if ($PSScriptRoot) { $startDirs += $PSScriptRoot }                                  # file launch (.cmd, -File, & '...')
    if ($MyInvocation.MyCommand -and $MyInvocation.MyCommand.Path) {                     # fallback: script path
        $startDirs += (Split-Path -Parent $MyInvocation.MyCommand.Path)
    }
    $startDirs += (Get-Location).Path                                                    # fallback: current working folder
    $Root = Find-AetherRoot -StartDirs $startDirs
} else {
    # Normalize an externally passed -Root (e.g. "C:\...\Tools\.." from the .cmd)
    try { $Root = [IO.Path]::GetFullPath($Root) } catch { $Root = $null }
}

if (-not $Root) {
    Write-Host "[ERROR] Cannot locate the AetherDLL folder." -ForegroundColor Red
    Write-Host "         Run the script from the repository root (the one holding AetherDLL\)"
    Write-Host "         or pass it explicitly:  .\sync_aetherdll.ps1 -Root 'C:\path\to\Aether'"
    exit 1
}

$dllDir    = Join-Path $Root 'AetherDLL'
$stateDir  = Join-Path $dllDir 'out'
$stateFile = Join-Path $stateDir '.aether_sync_state.json'

if (-not (Test-Path $dllDir)) {
    Write-Host "[ERROR] AetherDLL folder not found in: $Root" -ForegroundColor Red
    exit 1
}
if (-not (Test-Path $stateDir)) {
    New-Item -ItemType Directory -Path $stateDir | Out-Null
    Write-Host "[WARNING] out\ did not exist: the first build will be a full one." -ForegroundColor Yellow
}

Write-Host "Repository root: $Root"

# Sources to consider (build outputs and VCS metadata excluded)
$sep = [regex]::Escape([string][IO.Path]::DirectorySeparatorChar)
$files = Get-ChildItem -Path $dllDir -Recurse -File |
    Where-Object { $_.FullName -notmatch ($sep + 'out' + $sep) -and $_.FullName -notmatch ($sep + '\.git' + $sep) }

# Previous state: relative path -> SHA256
$prev = @{}
if (Test-Path $stateFile) {
    try { $prev = Get-Content $stateFile -Raw | ConvertFrom-Json } catch { $prev = @{} }
}

$now     = Get-Date
$touched = 0
$state   = [ordered]@{}

foreach ($f in $files) {
    $rel  = $f.FullName.Substring($dllDir.Length + 1)
    $hash = (Get-FileHash -Path $f.FullName -Algorithm SHA256).Hash
    $state[$rel] = $hash
    if ($prev.$rel -ne $hash) {
        $f.LastWriteTime = $now
        $touched++
        Write-Host ("  updated: " + $rel)
    }
}

$state | ConvertTo-Json | Set-Content -Path $stateFile -Encoding UTF8

Write-Host ""
Write-Host ("Source files scanned:        " + $files.Count)
Write-Host ("Timestamps refreshed:        " + $touched)
if ($touched -eq 0) {
    Write-Host "No content changes: 'Build All' will recompile nothing." -ForegroundColor Green
} else {
    Write-Host "Now in Visual Studio use: Build > Build All (NOT 'Rebuild All')." -ForegroundColor Green
}
