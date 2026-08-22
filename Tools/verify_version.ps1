# ============================================================================
# AetherDLL - Embedded version verification (single source of truth).
# Used BOTH by the local POST_BUILD gate in CMake and by the CI release job:
# if a built DLL lacks the version resource, or its version disagrees with the
# expected one, the build/release fails LOUDLY instead of shipping a binary
# whose version AetherDesk could never read.
#
#   powershell -File verify_version.ps1 -TargetFile <path.dll> -ExpectedVersion <x.y.z>
# ============================================================================
param(
    [Parameter(Mandatory = $true)][string]$TargetFile,
    [Parameter(Mandatory = $true)][string]$ExpectedVersion
)
$ErrorActionPreference = 'Stop'

$name = Split-Path $TargetFile -Leaf

if (!(Test-Path $TargetFile)) {
    Write-Error "[AetherDLL] file not found: $TargetFile"
    exit 1
}

$info = [System.Diagnostics.FileVersionInfo]::GetVersionInfo($TargetFile)
if ([string]::IsNullOrWhiteSpace($info.FileVersion)) {
    Write-Error "[AetherDLL] $name has NO embedded version resource (rc compile/link failed)"
    exit 1
}

$fileParts = ($info.FileVersion -split '\.')
$expectedParts = ($ExpectedVersion -split '\.')
$components = [Math]::Min(3, $expectedParts.Count)
for ($i = 0; $i -lt $components; $i++) {
    if ([int]$fileParts[$i] -ne [int]$expectedParts[$i]) {
        Write-Error "[AetherDLL] $name embedded version $($info.FileVersion) != expected $ExpectedVersion"
        exit 1
    }
}

Write-Host "[AetherDLL] $name -> embedded FileVersion $($info.FileVersion) OK"
exit 0
