# annot_unresolved.ps1 -- measure where Unresolved annotations come from (#15b payoff).
# Runs every test with AR_ANNOT_DIFF=1 and aggregates the AnnotUnresolvedSrc /
# AnnotBinop lines that arrow.exe prints on stderr after the static checks.
#
# Since 2026-11-24 (gate_speed_plan.md 8-1): the files run through the common runner (gate_run.ps1):
# in parallel, output to files, results saved by the exe's hash (-Fresh runs again). It used to run the
# examples one at a time (155.6 s -> 50.2 s, the same totals).
#
# ⚠ The default population stays examples/ (this is a measurement, not a pass/fail gate). On gate/
#   (-Root gate: the tests of Get-GateTestFiles) the counts are dominated by gate/lib/check.ar, which
#   almost every test imports: it alone adds 3 Unresolved operands and 19 mixed binops per run, so
#   gate/ gave 1475 Unresolved against 145 for examples/ (8-1).
#
# ⚠ AR_ANNOT_DIFF is not added to the shared suite run (Invoke-GateSuite): the lines would end up in the
#   stderr that compare_outputs compares, and a change in these internal counts would look like a change
#   in behaviour.
#
# The file has a UTF-8 BOM (it contains Japanese comments; PowerShell 5.1 reads BOM-less .ps1 as ANSI).

param(
    [int]$TimeoutSec = 20,
    [string]$Filter = '*.ar',
    [string]$Root = '',
    [int]$Jobs = 0,
    [switch]$Fresh
)

$ErrorActionPreference = 'Continue'
$repo = Split-Path -Parent $PSScriptRoot   # scripts/ の 1 つ上 = リポジトリ直下
$exe  = Join-Path $repo 'target\release\arrow.exe'
if (-not (Test-Path $exe)) { throw "not built: $exe" }
. (Join-Path $PSScriptRoot 'gate_run.ps1')
. (Join-Path $PSScriptRoot 'gate_judge.ps1')

if (-not $Root) { $Root = Join-Path $repo 'examples' }
$rootDir = (Resolve-Path $Root).Path.TrimEnd('\')
if ($rootDir -eq (Join-Path $repo 'examples')) {
    # Skips mirror _archive/run_examples.ps1 (env-dependent / interactive / generated artifacts).
    $skip = @('importation.ar', 'rs_crates', 'archived', 'cs_form', 'cs_proc', 'js_proc')
    $files = @(Get-ChildItem -Path $rootDir -Recurse -Filter $Filter |
        Where-Object { $p = $_.FullName; -not ($skip | Where-Object { $p -like "*$_*" }) })
} else {
    $files = @(Get-GateTestFiles $rootDir | Where-Object { $_.Name -like $Filter })
}
$files = @($files | Sort-Object FullName | ForEach-Object { $_.FullName })

$res = Invoke-GateRun -Exe $exe -Root $rootDir -Files $files -WorkDir file -ExtraEnv @{ AR_ANNOT_DIFF = '1' } `
    -TimeoutSec $TimeoutSec -Jobs $Jobs -Fresh:$Fresh

$srcTotals   = @{}
$binopTotals = @{ specialized = 0; both = 0; one = 0; mixed = 0 }
$ran = 0; $failed = 0

foreach ($r in $res) {
    if ($r.TimedOut) { $failed++; continue }
    $ran++
    foreach ($line in ((Read-GateText $r.ErrPath) -split "`r?`n")) {
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
