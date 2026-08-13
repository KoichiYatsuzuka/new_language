# ab_bench_vm.ps1 — ab_bench.ps1 の `--vm=<mode>` 付き版（#10-b の退行切り分け用）。
#
# ab_bench.ps1 は `--vm` を渡せないため、「退行が VM 経路由来か」を確かめられない。
# ⚠ **必ず交互実行する**こと。A を N 回→B を N 回の順で測ると、後に測った側が
#    サーマルドリフトで不利になり 10% 級の偽の差が出る（実際に一度誤認した）。
#
# Usage: ./ab_bench_vm.ps1 -A head.exe -B new.exe -Mode off -Scripts examples/bench/x.ar

param(
    [Parameter(Mandatory = $true)][string]$A,
    [Parameter(Mandatory = $true)][string]$B,
    [Parameter(Mandatory = $true)][ValidateSet('off', 'auto', 'force')][string]$Mode,
    [string[]]$Scripts = @(),
    [int]$Reps = 4
)

$ErrorActionPreference = 'Stop'
if ($Scripts.Count -eq 0) { $Scripts = Get-ChildItem "examples/bench/*.ar" | ForEach-Object { $_.FullName } }

function Measure-Run([string]$exe, [string]$script, [string]$mode) {
    $psi = New-Object System.Diagnostics.ProcessStartInfo
    $psi.FileName = (Resolve-Path $exe).Path
    $psi.Arguments = "-src `"$script`" --vm=$mode"
    $psi.UseShellExecute = $false
    $psi.RedirectStandardOutput = $true
    $psi.RedirectStandardError = $true
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    $p = [System.Diagnostics.Process]::Start($psi)
    [void]$p.StandardOutput.ReadToEnd()
    [void]$p.StandardError.ReadToEnd()
    $p.WaitForExit()
    $sw.Stop()
    return $sw.Elapsed.TotalSeconds
}

$rows = @()
foreach ($s in $Scripts) {
    $bestA = [double]::MaxValue
    $bestB = [double]::MaxValue
    for ($i = 0; $i -lt $Reps; $i++) {
        # 交互実行（A,B,A,B,...）でドリフトを両者に等しく配分する。
        $ta = Measure-Run $A $s $Mode; if ($ta -lt $bestA) { $bestA = $ta }
        $tb = Measure-Run $B $s $Mode; if ($tb -lt $bestB) { $bestB = $tb }
    }
    $rows += [pscustomobject]@{
        script = (Split-Path $s -Leaf)
        'A(min)' = [math]::Round($bestA, 3)
        'B(min)' = [math]::Round($bestB, 3)
        'A/B' = "{0:N3}x" -f ($bestA / $bestB)
    }
}
$rows | Format-Table -AutoSize
