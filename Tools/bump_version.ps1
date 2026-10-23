param(
    [Parameter(Mandatory = $true)]
    [ValidateSet('Desk', 'Dll')]
    [string]$Component,

    [Parameter(Mandatory = $true)]
    [string]$Version,

    [switch]$DryRun
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
if (-not (Test-Path -LiteralPath (Join-Path $root 'AetherDesk'))) {
    throw "Repository root not found from $PSScriptRoot"
}

if ($Version -notmatch '^\d+\.\d+\.\d+$') {
    throw "Invalid version '$Version': expected X.Y.Z (for example 1.5.0)."
}

$utf8 = New-Object System.Text.UTF8Encoding($false)
$label = if ($Component -eq 'Desk') { 'AetherDesk' } else { 'AetherDLL' }

# Replaces exactly the captured fields, never every "version" occurrence in the
# file: package-lock.json contains hundreds of dependency versions.
function Set-VersionInText {
    param(
        [string]$Text,
        [string]$Pattern,
        [string]$NewVersion,
        [int]$Expected,
        [string]$Label
    )

    $regex = [regex]::new($Pattern, [System.Text.RegularExpressions.RegexOptions]::Multiline)
    $count = $regex.Matches($Text).Count
    if ($count -ne $Expected) {
        throw "${Label}: expected $Expected version field(s), found $count. Nothing was written."
    }
    $replacement = '${1}' + $NewVersion + '${2}'
    return $regex.Replace($Text, $replacement)
}

function New-Plan {
    param(
        [string]$RelativePath,
        [string]$Pattern,
        [int]$Expected,
        [string]$Label
    )

    $path = Join-Path $root $RelativePath
    if (-not (Test-Path -LiteralPath $path)) {
        throw "File not found: $RelativePath"
    }
    $text = [System.IO.File]::ReadAllText($path)
    $updated = Set-VersionInText -Text $text -Pattern $Pattern -NewVersion $Version -Expected $Expected -Label $Label
    return [pscustomobject]@{ Path = $path; Relative = $RelativePath; Text = $updated }
}

function Get-CurrentVersion {
    if ($Component -eq 'Desk') {
        $text = [System.IO.File]::ReadAllText((Join-Path $root 'AetherDesk/src-tauri/Cargo.toml'))
        $match = [regex]::Match($text, '(?m)^version\s*=\s*"([^"]+)"')
    } else {
        $text = [System.IO.File]::ReadAllText((Join-Path $root 'AetherDLL/CMakeLists.txt'))
        $match = [regex]::Match($text, 'project\(AetherDLL LANGUAGES C CXX RC VERSION ([0-9.]+)\)')
    }
    if ($match.Success) {
        return $match.Groups[1].Value
    }
    return 'unknown'
}

$planned = @()

if ($Component -eq 'Desk') {
    $planned += New-Plan 'AetherDesk/package.json' '(?m)^(\s*"version"\s*:\s*")[^"]+(")' 1 'package.json'
    $planned += New-Plan 'AetherDesk/src-tauri/tauri.conf.json' '(?m)^(\s*"version"\s*:\s*")[^"]+(")' 1 'tauri.conf.json'
    $planned += New-Plan 'AetherDesk/src-tauri/Cargo.toml' '(?m)^(version\s*=\s*")[^"]+(")' 1 'Cargo.toml'
    $planned += New-Plan 'AetherDesk/src-tauri/Cargo.lock' '(?ms)(\[\[package\]\]\r?\nname = "aether_desk"\r?\nversion = ")[^"]+(")' 1 'Cargo.lock'

    $lockPath = Join-Path $root 'AetherDesk/package-lock.json'
    $lockText = [System.IO.File]::ReadAllText($lockPath)
    $lockText = Set-VersionInText -Text $lockText -Pattern '(?m)\A(\{\r?\n  "name": "aetherdesk-ui",\r?\n  "version": ")[^"]+(")' -NewVersion $Version -Expected 1 -Label 'package-lock.json root'
    $lockText = Set-VersionInText -Text $lockText -Pattern '(?m)("packages": \{\r?\n    "": \{\r?\n      "name": "aetherdesk-ui",\r?\n      "version": ")[^"]+(")' -NewVersion $Version -Expected 1 -Label 'package-lock.json package'
    $planned += [pscustomobject]@{ Path = $lockPath; Relative = 'AetherDesk/package-lock.json'; Text = $lockText }
} else {
    $planned += New-Plan 'AetherDLL/CMakeLists.txt' '(?m)^(project\(AetherDLL LANGUAGES C CXX RC VERSION )[0-9]+\.[0-9]+\.[0-9]+(\))' 1 'AetherDLL/CMakeLists.txt'
}

Write-Host ("{0}: {1} -> {2}" -f $label, (Get-CurrentVersion), $Version)

if ($DryRun) {
    foreach ($change in $planned) {
        Write-Host "  would update $($change.Relative)"
    }
    Write-Host 'Dry run: no file was written.'
    return
}

foreach ($change in $planned) {
    [System.IO.File]::WriteAllText($change.Path, $change.Text, $utf8)
}

foreach ($change in $planned) {
    if ([System.IO.File]::ReadAllText($change.Path) -ne $change.Text) {
        throw "Write verification failed: $($change.Relative)"
    }
    Write-Host "  updated $($change.Relative)"
}

Write-Host 'Done.'
