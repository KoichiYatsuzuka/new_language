# ab_bench.ps1 — 2 つの arrow.exe を交互に走らせて経過時間を比較する（A/B 計測）
#
# 計画書の「A/B は当該変更だけを切り替えて取ること」用。HEAD 版と変更版のバイナリを
# 別々に用意し（`git stash push -- src/` でビルドして退避）、同一マシン・同一時間帯で
# 交互実行する。前回測定から時間が空いた値と比べると他要因を誤って帰属するため。
#
# Usage:
#   ./scripts/ab_bench.ps1 -A <head.exe> -B <new.exe> -Scripts examples/bench/bench_for.ar,examples/bench/bench_arith.ar
#   ./scripts/ab_bench.ps1 -A a.exe -B b.exe -Reps 5
#   ./scripts/ab_bench.ps1 -A a.exe -B b.exe -TimeoutSec 300
#
# 注意: Start-Process -PassThru の ExitCode は当てにならないので System.Diagnostics.Process を直接使う。

param(
    [Parameter(Mandatory = $true)][string]$A,
    [Parameter(Mandatory = $true)][string]$B,
    [string[]]$Scripts = @(),
    [int]$Reps = 3,
    [int]$TimeoutSec = 180
)

$ErrorActionPreference = "Stop"
$repo = Split-Path -Parent $PSScriptRoot   # scripts/ の 1 つ上 = リポジトリ直下

if ($Scripts.Count -eq 0) {
    # 既定はリポジトリ直下基準（カレントディレクトリ依存で空集合にならないように）
    $Scripts = Get-ChildItem (Join-Path $repo 'examples/bench/*.ar') | ForEach-Object { $_.FullName }
}

# 1 回実行して経過秒を返す（stdout/stderr は捨てる）。
#
# ⚠ ReadToEnd() を stdout → stderr の順に逐次呼んではいけない（#38）。子が stderr の
#   パイプを埋めると子は書き込みでブロックし、親は stdout の EOF を待ち続けて相互に
#   固まる（#34 で 1 時間停止）。症状は「CPU 時間が伸びないまま生き続ける」。
#   必ず ReadToEndAsync() で両方を同時に読むこと（scan_examples.ps1 と同じ形）。
#   ⚠ 再現条件は「子が数 KB 以上の stderr を吐くこと」なので、AR_VM_DUMP=1 のような
#     診断フックを付けた瞬間に初めて踏む。普段の実行で通っても直った証拠にはならない。
#
# 戻り値: [pscustomobject] @{ Seconds; Ok; Reason }
#   Ok=$false のとき Seconds は意味を持たない（タイムアウト／異常終了した実行を
#   min に混ぜると「速くなった」と誤読するため、呼び出し側で除外する）。
function Measure-Run([string]$exe, [string]$script) {
    $psi = New-Object System.Diagnostics.ProcessStartInfo
    $psi.FileName = (Resolve-Path $exe).Path
    $psi.Arguments = "-src `"$script`""
    $psi.WorkingDirectory = $repo
    $psi.RedirectStandardOutput = $true
    $psi.RedirectStandardError = $true
    $psi.UseShellExecute = $false
    $psi.CreateNoWindow = $true

    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    $p = [System.Diagnostics.Process]::Start($psi)
    # 出力バッファ充填でのデッドロックを避けるため両方を同時に非同期で読む
    $outTask = $p.StandardOutput.ReadToEndAsync()
    $errTask = $p.StandardError.ReadToEndAsync()

    if (-not $p.WaitForExit($TimeoutSec * 1000)) {
        try { $p.Kill() } catch {}
        try { $p.WaitForExit(5000) | Out-Null } catch {}
        $p.Dispose()
        return [pscustomobject]@{ Seconds = 0.0; Ok = $false; Reason = "TIMEOUT(${TimeoutSec}s)" }
    }
    $sw.Stop()
    $code = $p.ExitCode
    # 子が終了した後なので読み切りは詰まらない
    $err = $errTask.Result
    $outTask.Result | Out-Null
    $p.Dispose()

    if ($code -ne 0) {
        $line = ($err -split "`n" | Where-Object { $_ -match '\S' } | Select-Object -Last 1)
        $line = (($line -replace '\x1b\[[0-9;]*m', '') -replace '\s+', ' ').Trim()
        return [pscustomobject]@{ Seconds = $sw.Elapsed.TotalSeconds; Ok = $false; Reason = "EXIT=$code $line" }
    }
    return [pscustomobject]@{ Seconds = $sw.Elapsed.TotalSeconds; Ok = $true; Reason = '' }
}

Write-Host ("{0,-38} {1,9} {2,9} {3,8}" -f "script", "A(min)", "B(min)", "A/B")

$failed = @()
foreach ($s in $Scripts) {
    if (-not (Test-Path $s)) {
        # 黙って飛ばさない。飛ばすと「表が空のまま exit 0」になり計測したつもりで何も測れない
        # （`powershell -File` 経由だと -Scripts a,b,c が 1 要素の文字列として渡るのが典型）
        Write-Host ("{0,-38} {1,9} {2,9} {3,8}   {4}" -f (Split-Path $s -Leaf), "-", "-", "-", "NOT FOUND: $s")
        $failed += $s
        continue
    }
    $name = Split-Path $s -Leaf
    $ta = @(); $tb = @(); $bad = @()
    for ($r = 1; $r -le $Reps; $r++) {
        # 交互に実行してマシン変動を両者へ均等に散らす
        $ra = Measure-Run $A $s
        if ($ra.Ok) { $ta += $ra.Seconds } else { $bad += "A: $($ra.Reason)" }
        $rb = Measure-Run $B $s
        if ($rb.Ok) { $tb += $rb.Seconds } else { $bad += "B: $($rb.Reason)" }
    }
    if ($ta.Count -eq 0 -or $tb.Count -eq 0) {
        # 片側が 1 回も成功していない ＝ 比較不能。黙って飛ばさず理由を出す
        Write-Host ("{0,-38} {1,9} {2,9} {3,8}   {4}" -f $name, "-", "-", "-", (($bad | Select-Object -Unique) -join ' / '))
        $failed += $name
        continue
    }
    $minA = ($ta | Measure-Object -Minimum).Minimum
    $minB = ($tb | Measure-Object -Minimum).Minimum
    $ratio = if ($minB -gt 0) { $minA / $minB } else { 0 }
    $note = if ($bad.Count -gt 0) { "   !! " + (($bad | Select-Object -Unique) -join ' / ') } else { "" }
    Write-Host ("{0,-38} {1,9:N3} {2,9:N3} {3,8:N3}x{4}" -f $name, $minA, $minB, $ratio, $note)
    if ($bad.Count -gt 0) { $failed += $name }
}

if ($failed.Count -gt 0) {
    Write-Host ""
    Write-Host ("WARN: 失敗した実行がある（値は信用しない）: " + (($failed | Select-Object -Unique) -join ', '))
}
