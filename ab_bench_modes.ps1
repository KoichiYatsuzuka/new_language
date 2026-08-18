# ab_bench_modes.ps1 — 2 つの arrow.exe を **3 つの実行モード**で交互に比較する A/B 計測
#
#   モード1 非コンパイル      : examples/bench/bench_ab_interp.ar
#   モード2 arrow native      : examples/bench/bench_ab_native.ar（+ --compile した .arc）
#   モード3 C の DLL          : examples/interop/bench_ab_cdll.ar（import[cpp-lib]）
#
# Usage:
#   powershell -Command "./ab_bench_modes.ps1 -A <master.exe> -B <bytecode.exe> -Reps 3"
#   powershell -Command "./ab_bench_modes.ps1 -A a.exe -B b.exe -Modes interp,cdll"
#
# ab_bench.ps1 との違い:
#   * プロセス全体の経過時間ではなく、スクリプトが出す `METRIC <name> <secs>` を解析する
#     （起動・DLL ロード・リスト構築などの固定費を計測から外すため）。
#   * モード2 は **測る側のバイナリで --compile し直してから**走らせる
#     （.arc の形式がブランチ間で違うので、片方の .arc を他方が読むと比較にならない）。
#   * A/B の `CHECKSUM` 行を突き合わせ、食い違ったら値を出さずに警告する
#     （速くなったのではなく計算をしていないだけ、を見逃さないため）。
#
# 注意（既知の落とし穴）:
#   * Start-Process -PassThru の ExitCode は当てにならないので System.Diagnostics.Process を直接使う。
#   * stdout/stderr は必ず ReadToEndAsync() で同時に読む（逐次 ReadToEnd() は子とデッドロックする）。
#   * `powershell -File` 経由だと -Modes a,b が 1 要素に潰れるので -Command で呼ぶこと。

param(
    [Parameter(Mandatory = $true)][string]$A,
    [Parameter(Mandatory = $true)][string]$B,
    [int]$Reps = 3,
    [int]$TimeoutSec = 600,
    [string[]]$Modes = @('interp', 'native', 'cdll'),
    [string]$Csv = ''
)

$ErrorActionPreference = 'Stop'
$repo = $PSScriptRoot

$SPECS = @(
    [pscustomobject]@{ Mode = 'interp'; Script = 'examples/bench/bench_ab_interp.ar';  Compile = '' }
    [pscustomobject]@{ Mode = 'native'; Script = 'examples/bench/bench_ab_native.ar';  Compile = 'examples/bench/bench_ab_native_module.ar' }
    [pscustomobject]@{ Mode = 'cdll';   Script = 'examples/interop/bench_ab_cdll.ar';  Compile = '' }
)

# 子プロセスを 1 回走らせて stdout/stderr/exit code を返す
function Invoke-Arrow([string]$exe, [string]$argLine) {
    $psi = New-Object System.Diagnostics.ProcessStartInfo
    $psi.FileName = (Resolve-Path $exe).Path
    $psi.Arguments = $argLine
    $psi.WorkingDirectory = $repo
    $psi.RedirectStandardOutput = $true
    $psi.RedirectStandardError = $true
    $psi.UseShellExecute = $false
    $psi.CreateNoWindow = $true

    $p = [System.Diagnostics.Process]::Start($psi)
    # 出力バッファ充填でのデッドロックを避けるため両方を同時に非同期で読む
    $outTask = $p.StandardOutput.ReadToEndAsync()
    $errTask = $p.StandardError.ReadToEndAsync()
    if (-not $p.WaitForExit($TimeoutSec * 1000)) {
        try { $p.Kill() } catch {}
        try { $p.WaitForExit(5000) | Out-Null } catch {}
        $p.Dispose()
        return [pscustomobject]@{ Ok = $false; Out = ''; Reason = "TIMEOUT(${TimeoutSec}s)" }
    }
    $code = $p.ExitCode
    $out = $outTask.Result
    $err = $errTask.Result
    $p.Dispose()
    if ($code -ne 0) {
        $line = ($err -split "`n" | Where-Object { $_ -match '\S' } | Select-Object -Last 1)
        $line = (($line -replace '\x1b\[[0-9;]*m', '') -replace '\s+', ' ').Trim()
        return [pscustomobject]@{ Ok = $false; Out = $out; Reason = "EXIT=$code $line" }
    }
    return [pscustomobject]@{ Ok = $true; Out = $out; Reason = '' }
}

# `METRIC <name> <secs>` / `CHECKSUM <v>` を拾う
function Read-Metrics([string]$out) {
    $m = [ordered]@{}
    $sum = $null
    foreach ($line in ($out -split "`r?`n")) {
        if ($line -match '^\s*METRIC\s+(\S+)\s+(\S+)\s*$') { $m[$Matches[1]] = [double]$Matches[2] }
        elseif ($line -match '^\s*CHECKSUM\s+(\S+)\s*$')   { $sum = $Matches[1] }
    }
    return [pscustomobject]@{ Metrics = $m; Checksum = $sum }
}

# 1 side / 1 rep: 必要なら --compile してから計測実行
function Measure-Side([string]$exe, $spec) {
    if ($spec.Compile -ne '') {
        $c = Invoke-Arrow $exe "--compile `"$($spec.Compile)`""
        if (-not $c.Ok) { return [pscustomobject]@{ Ok = $false; Reason = "compile: $($c.Reason)" } }
    }
    $r = Invoke-Arrow $exe "-src `"$($spec.Script)`""
    if (-not $r.Ok) { return [pscustomobject]@{ Ok = $false; Reason = $r.Reason } }
    $parsed = Read-Metrics $r.Out
    if ($parsed.Metrics.Count -eq 0) {
        # 黙って空表を出さない（計測したつもりで何も測れていない典型）
        return [pscustomobject]@{ Ok = $false; Reason = 'NO METRIC LINES' }
    }
    return [pscustomobject]@{ Ok = $true; Metrics = $parsed.Metrics; Checksum = $parsed.Checksum; Reason = '' }
}

$rows = @()
$problems = @()

foreach ($spec in $SPECS) {
    if ($Modes -notcontains $spec.Mode) { continue }
    $path = Join-Path $repo $spec.Script
    if (-not (Test-Path $path)) {
        $problems += "$($spec.Mode): NOT FOUND $($spec.Script)"
        continue
    }

    Write-Host ""
    Write-Host "==== mode: $($spec.Mode)  ($($spec.Script)) ====" -ForegroundColor Cyan

    $accA = @{}; $accB = @{}; $order = @()
    $ckA = $null; $ckB = $null

    for ($r = 1; $r -le $Reps; $r++) {
        Write-Host ("-- rep {0}/{1} --" -f $r, $Reps) -ForegroundColor DarkGray
        # 交互に実行してマシン変動を両者へ均等に散らす
        foreach ($side in @('A', 'B')) {
            $exe = if ($side -eq 'A') { $A } else { $B }
            $res = Measure-Side $exe $spec
            if (-not $res.Ok) { $problems += "$($spec.Mode) $side rep$r : $($res.Reason)"; continue }
            $acc = if ($side -eq 'A') { $accA } else { $accB }
            foreach ($k in $res.Metrics.Keys) {
                if ($order -notcontains $k) { $order += $k }
                if (-not $acc.ContainsKey($k)) { $acc[$k] = @() }
                $acc[$k] += $res.Metrics[$k]
            }
            if ($side -eq 'A') { $ckA = $res.Checksum } else { $ckB = $res.Checksum }
        }
    }

    if ($ckA -ne $null -and $ckB -ne $null -and $ckA -ne $ckB) {
        # 「速い」のではなく「同じ計算をしていない」を見逃さない
        $problems += "$($spec.Mode): CHECKSUM MISMATCH A=$ckA B=$ckB"
    }

    Write-Host ("{0,-30} {1,10} {2,10} {3,9}" -f 'metric', 'A(min s)', 'B(min s)', 'A/B')
    foreach ($k in $order) {
        if (-not $accA.ContainsKey($k) -or -not $accB.ContainsKey($k)) {
            Write-Host ("{0,-30} {1,10} {2,10} {3,9}   {4}" -f $k, '-', '-', '-', 'missing on one side')
            continue
        }
        $ma = ($accA[$k] | Measure-Object -Minimum).Minimum
        $mb = ($accB[$k] | Measure-Object -Minimum).Minimum
        $ratio = if ($mb -gt 0) { $ma / $mb } else { 0 }
        Write-Host ("{0,-30} {1,10:N4} {2,10:N4} {3,9:N3}x" -f $k, $ma, $mb, $ratio)
        $rows += [pscustomobject]@{ Mode = $spec.Mode; Metric = $k; A = $ma; B = $mb; Ratio = $ratio }
    }
}

if ($Csv -ne '') {
    $rows | Export-Csv -NoTypeInformation -Encoding UTF8 -Path $Csv
    Write-Host ""
    Write-Host "csv: $Csv"
}

if ($problems.Count -gt 0) {
    Write-Host ""
    Write-Host "WARN: 失敗/不整合がある（該当行の値は信用しない）:" -ForegroundColor Yellow
    foreach ($p in ($problems | Select-Object -Unique)) { Write-Host "  $p" -ForegroundColor Yellow }
}
