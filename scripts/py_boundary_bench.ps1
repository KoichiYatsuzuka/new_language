# py_boundary_bench.ps1 - measures the Python <-> Arrow boundary (pandas_import_plan.md 4-1 / 4-2 / 4-3).
#
# Each row of scripts/py_boundary/bench.ar is ns per element (or per call), the minimum of its rounds. With -B the two
# exes run alternately, and the last column is B - A.
#
#   ./scripts/py_boundary_bench.ps1                      # target/release/arrow.exe only
#   ./scripts/py_boundary_bench.ps1 -A base.exe -B cur.exe -Repeat 3
#
# Take a negative control first (-A x.exe -B x.exe): the noise band differs from session to session (vm-pitfalls 1).
# The rows "loop only", "for list[int]" and "index list[int]" do not touch Python, so they show the noise of the session.
param(
    [string]$A = 'target\release\arrow.exe',
    [string]$B = '',
    [int]$Repeat = 2
)
$root = Split-Path -Parent $PSScriptRoot
$bench = Join-Path $PSScriptRoot 'py_boundary\bench.ar'

function Resolve-Exe([string]$exe) {
    if ([System.IO.Path]::IsPathRooted($exe)) { return $exe }
    return Join-Path $root $exe
}

function Run-Once([string]$exe, [hashtable]$table, [System.Collections.ArrayList]$order) {
    Push-Location (Split-Path -Parent $bench)
    try {
        $lines = & $exe 'bench.ar' 2>&1
    } finally {
        Pop-Location
    }
    foreach ($line in $lines) {
        $parts = "$line" -split "`t"
        if ($parts.Count -ne 2) {
            if ("$line".Trim() -ne '') { Write-Host "  $line" }
            continue
        }
        $name = $parts[0]
        $value = [double]$parts[1]
        if (-not $table.ContainsKey($name)) {
            $table[$name] = $value
            if (-not $order.Contains($name)) { [void]$order.Add($name) }
        } elseif ($value -lt $table[$name]) {
            $table[$name] = $value
        }
    }
}

$exeA = Resolve-Exe $A
$exeB = if ($B) { Resolve-Exe $B } else { '' }
$order = New-Object System.Collections.ArrayList
$tableA = @{}
$tableB = @{}
for ($i = 0; $i -lt $Repeat; $i++) {
    Run-Once $exeA $tableA $order
    if ($exeB) { Run-Once $exeB $tableB $order }
}

if ($exeB) {
    Write-Host ('{0,-30}{1,10}{2,10}{3,10}' -f 'row (ns)', 'A', 'B', 'B-A')
    foreach ($name in $order) {
        $x = $tableA[$name]
        $y = $tableB[$name]
        Write-Host ('{0,-30}{1,10:F1}{2,10:F1}{3,10:F1}' -f $name, $x, $y, ($y - $x))
    }
} else {
    Write-Host ('{0,-30}{1,10}' -f 'row (ns)', 'A')
    foreach ($name in $order) {
        Write-Host ('{0,-30}{1,10:F1}' -f $name, $tableA[$name])
    }
}
