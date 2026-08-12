# ab_bench.ps1 — 2 つの arrow.exe を交互に走らせて経過時間を比較する（A/B 計測）
#
# 計画書の「A/B は当該変更だけを切り替えて取ること」用。HEAD 版と変更版のバイナリを
# 別々に用意し（`git stash push -- src/` でビルドして退避）、同一マシン・同一時間帯で
# 交互実行する。前回測定から時間が空いた値と比べると他要因を誤って帰属するため。
#
# Usage:
#   ./ab_bench.ps1 -A <head.exe> -B <new.exe> -Scripts examples/bench/bench_for.ar,examples/bench/bench_arith.ar
#   ./ab_bench.ps1 -A a.exe -B b.exe -Reps 5
#
# 注意: Start-Process -PassThru の ExitCode は当てにならないので System.Diagnostics.Process を直接使う。

param(
    [Parameter(Mandatory = $true)][string]$A,
    [Parameter(Mandatory = $true)][string]$B,
    [string[]]$Scripts = @(),
    [int]$Reps = 3
)

$ErrorActionPreference = "Stop"

if ($Scripts.Count -eq 0) {
    $Scripts = Get-ChildItem "examples/bench/*.ar" | ForEach-Object { $_.FullName }
}

# 1 回実行して経過秒を返す（stdout/stderr は捨てる）。
function Measure-Run([string]$exe, [string]$script) {
    $psi = New-Object System.Diagnostics.ProcessStartInfo
    $psi.FileName = (Resolve-Path $exe).Path
    $psi.Arguments = "-src `"$script`""
    $psi.RedirectStandardOutput = $true
    $psi.RedirectStandardError = $true
    $psi.UseShellExecute = $false
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    $p = [System.Diagnostics.Process]::Start($psi)
    $p.StandardOutput.ReadToEnd() | Out-Null
    $p.StandardError.ReadToEnd() | Out-Null
    $p.WaitForExit()
    $sw.Stop()
    return $sw.Elapsed.TotalSeconds
}

Write-Host ("{0,-38} {1,9} {2,9} {3,8}" -f "script", "A(min)", "B(min)", "A/B")

foreach ($s in $Scripts) {
    if (-not (Test-Path $s)) { continue }
    $name = Split-Path $s -Leaf
    $ta = @(); $tb = @()
    for ($r = 1; $r -le $Reps; $r++) {
        # 交互に実行してマシン変動を両者へ均等に散らす
        $ta += Measure-Run $A $s
        $tb += Measure-Run $B $s
    }
    $minA = ($ta | Measure-Object -Minimum).Minimum
    $minB = ($tb | Measure-Object -Minimum).Minimum
    $ratio = if ($minB -gt 0) { $minA / $minB } else { 0 }
    Write-Host ("{0,-38} {1,9:N3} {2,9:N3} {3,8:N3}x" -f $name, $minA, $minB, $ratio)
}
