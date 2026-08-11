# annot_unresolved.ps1 -- measure where Unresolved annotations come from (#15b payoff).
# Runs every example with AR_ANNOT_DIFF=1 and aggregates the AnnotUnresolvedSrc /
# AnnotBinop lines that the type checker emits on stderr.
#
# ASCII-only on purpose: PowerShell 5.1 reads BOM-less .ps1 as ANSI.

param(
    [int]$TimeoutSec = 20,
    [string]$Filter = '*.ar'
)

$ErrorActionPreference = 'Continue'
$repo = $PSScriptRoot
$exe  = Join-Path $repo 'target\release\arrow.exe'
if (-not (Test-Path $exe)) { throw "not built: $exe" }

# Skips mirror run_examples.ps1 (env-dependent / interactive / generated artifacts).
$skip = @('importation.ar', 'rs_crates', 'archived', 'cs_form', 'cs_proc', 'js_proc')

$files = Get-ChildItem -Path (Join-Path $repo 'examples') -Recurse -Filter $Filter |
    Where-Object { $p = $_.FullName; -not ($skip | Where-Object { $p -like "*$_*" }) }

$srcTotals   = @{}
$binopTotals = @{ specialized = 0; both = 0; one = 0; mixed = 0 }
$ran = 0; $failed = 0

foreach ($f in $files) {
    $psi = New-Object System.Diagnostics.ProcessStartInfo
    $psi.FileName  = $exe
    $psi.Arguments = "-src `"$($f.FullName)`""
    $psi.WorkingDirectory     = $f.DirectoryName
    $psi.RedirectStandardError  = $true
    $psi.RedirectStandardOutput = $true
    $psi.UseShellExecute = $false
    $psi.EnvironmentVariables['AR_ANNOT_DIFF'] = '1'

    $proc = New-Object System.Diagnostics.Process
    $proc.StartInfo = $psi
    [void]$proc.Start()
    $errTask = $proc.StandardError.ReadToEndAsync()
    $outTask = $proc.StandardOutput.ReadToEndAsync()

    if (-not $proc.WaitForExit($TimeoutSec * 1000)) {
        try { $proc.Kill() } catch {}
        $failed++
        continue
    }
    $stderr = $errTask.Result
    [void]$outTask.Result
    $ran++

    foreach ($line in ($stderr -split "`r?`n")) {
        if ($line -match '^AnnotUnresolvedSrc:\s*(.*)$') {
            foreach ($pair in ($matches[1] -split '\s+')) {
                if ($pair -match '^(.+)=(\d+)$') {
                    $k = $matches[1]; $v = [int]$matches[2]
                    if (-not $srcTotals.ContainsKey($k)) { $srcTotals[$k] = 0 }
                    $srcTotals[$k] += $v
                }
            }
        }
        elseif ($line -match '^AnnotBinop: specialized=(\d+) miss_both_unresolved=(\d+) miss_one_unresolved=(\d+) miss_resolved_mixed=(\d+)') {
            $binopTotals.specialized += [int]$matches[1]
            $binopTotals.both        += [int]$matches[2]
            $binopTotals.one         += [int]$matches[3]
            $binopTotals.mixed       += [int]$matches[4]
        }
    }
}

Write-Host ""
Write-Host "=== examples run: $ran (timed out/skipped: $failed) ==="
Write-Host ""
Write-Host "--- Unresolved sources (expr kind that produced an Unresolved operand) ---"
$total = ($srcTotals.Values | Measure-Object -Sum).Sum
if (-not $total) { $total = 0 }
foreach ($e in ($srcTotals.GetEnumerator() | Sort-Object Value -Descending)) {
    $pct = if ($total -gt 0) { [math]::Round(100.0 * $e.Value / $total, 1) } else { 0 }
    "{0,-14} {1,6}  {2,5}%" -f $e.Key, $e.Value, $pct | Write-Host
}
"{0,-14} {1,6}" -f 'TOTAL', $total | Write-Host
Write-Host ""
Write-Host "--- Binop specialization ---"
"specialized      {0}" -f $binopTotals.specialized | Write-Host
"miss both        {0}" -f $binopTotals.both        | Write-Host
"miss one         {0}" -f $binopTotals.one         | Write-Host
"miss mixed       {0}" -f $binopTotals.mixed       | Write-Host
