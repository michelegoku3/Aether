# ============================================================================
# sync_aetherdll.ps1 - Aggiorna i timestamp dei sorgenti AetherDLL
# ----------------------------------------------------------------------------
# PERCHE' SERVE:
#   I file scaricati da GitHub/browser agent hanno come "data di modifica"
#   l'ora del commit, spesso PIU' VECCHIA degli oggetti gia' compilati in
#   AetherDLL\out\. Il copia-incolla di Windows conserva quella data: ninja
#   quindi crede che nulla sia cambiato e "Compila tutto" non ricompila i
#   file sostituiti (da qui l'obbligo di usare "Ricompila tutto").
#
# COSA FA:
#   Confronta l'HASH del contenuto di ogni file sorgente con quello dell'
#   ultima esecuzione e aggiorna il timestamp SOLO dei file davvero cambiati.
#   Dopo questo script, "Genera > Compila tutto" ricompila esattamente i file
#   modificati dall'agente AI e nient'altro (protobuf/lua/minhook restano
#   intatti perche' vivono in out\ e non vengono toccati).
#
# USO:
#   1) Sostituisci i file in AetherDLL (copia-incolla, SENZA cancellare la
#      cartella out\).
#   2) Esegui questo script (doppio click su sync_aetherdll.cmd).
#   3) In Visual Studio: Genera > Compila tutto  (NON "Ricompila tutto").
#
# NOTA TECNICA:
#   La radice della repository NON viene risolta nei valori di default di
#   param(): in Windows PowerShell 5.1 $PSScriptRoot li' e' una stringa vuota
#   (viene popolato solo a script avviato). La risoluzione avviene nel corpo
#   dello script con piu' fallback, cosi' funziona con qualsiasi modalita' di
#   avvio (doppio click, "Esegui con PowerShell", terminale, ISE).
# ============================================================================

[CmdletBinding()]
param(
    # Opzionale: radice della repository. Se omessa, viene rilevata da sola.
    [string]$Root = ''
)

$ErrorActionPreference = 'Stop'

function Find-AetherRoot {
    param([string[]]$StartDirs)
    foreach ($dir in $StartDirs) {
        if ([string]::IsNullOrWhiteSpace($dir)) { continue }
        try { $dir = [IO.Path]::GetFullPath($dir) } catch { continue }
        # Risali l'albero (max 5 livelli) cercando una cartella che contenga AetherDLL
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
    if ($PSScriptRoot) { $startDirs += $PSScriptRoot }                                  # avvio da file (.cmd, -File, & '...')
    if ($MyInvocation.MyCommand -and $MyInvocation.MyCommand.Path) {                     # fallback: percorso dello script
        $startDirs += (Split-Path -Parent $MyInvocation.MyCommand.Path)
    }
    $startDirs += (Get-Location).Path                                                    # fallback: cartella di lavoro corrente
    $Root = Find-AetherRoot -StartDirs $startDirs
} else {
    # Normalizza un -Root passato dall'esterno (es. "C:\...\Tools\.." dal .cmd)
    try { $Root = [IO.Path]::GetFullPath($Root) } catch { $Root = $null }
}

if (-not $Root) {
    Write-Host "[ERRORE] Non riesco a individuare la cartella AetherDLL." -ForegroundColor Red
    Write-Host "         Esegui lo script dalla radice della repository (quella che contiene AetherDLL\)"
    Write-Host "         oppure passala esplicitamente:  .\sync_aetherdll.ps1 -Root 'C:\path\to\Aether'"
    exit 1
}

$dllDir    = Join-Path $Root 'AetherDLL'
$stateDir  = Join-Path $dllDir 'out'
$stateFile = Join-Path $stateDir '.aether_sync_state.json'

if (-not (Test-Path $dllDir)) {
    Write-Host "[ERRORE] Cartella AetherDLL non trovata in: $Root" -ForegroundColor Red
    exit 1
}
if (-not (Test-Path $stateDir)) {
    New-Item -ItemType Directory -Path $stateDir | Out-Null
    Write-Host "[AVVISO] out\ non esisteva: la prima compilazione sara' completa." -ForegroundColor Yellow
}

Write-Host "Radice repository: $Root"

# Sorgenti da considerare (esclusi output di build e metadati VCS)
$sep = [regex]::Escape([string][IO.Path]::DirectorySeparatorChar)
$files = Get-ChildItem -Path $dllDir -Recurse -File |
    Where-Object { $_.FullName -notmatch ($sep + 'out' + $sep) -and $_.FullName -notmatch ($sep + '\.git' + $sep) }

# Stato precedente: percorso relativo -> SHA256
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
        Write-Host ("  aggiornato: " + $rel)
    }
}

$state | ConvertTo-Json | Set-Content -Path $stateFile -Encoding UTF8

Write-Host ""
Write-Host ("File sorgente analizzati:      " + $files.Count)
Write-Host ("Timestamp aggiornati:          " + $touched)
if ($touched -eq 0) {
    Write-Host "Nessuna modifica di contenuto: 'Compila tutto' non ricompilera' nulla." -ForegroundColor Green
} else {
    Write-Host "Ora in Visual Studio usa: Genera > Compila tutto (NON 'Ricompila tutto')." -ForegroundColor Green
}
