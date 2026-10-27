# native_slowdown.ps1 — コンパイルすると VM より遅くなる操作を、部分に分けて測る（native_slowdown_analysis.md）
#
#   ./scripts/native_slowdown.ps1                          # target/release の exe で、非コンパイルとコンパイル済みを比べる
#   ./scripts/native_slowdown.ps1 -Exe <exe> -Reps 7       # exe・回数を変える
#   ./scripts/native_slowdown.ps1 -Exe <a.exe> -B <b.exe>  # 2 つ目の exe のコンパイル済みの列を足す（実験用の exe と比べる）
#
# 仕組み:
#   * examples/bench/speed_modes/slowdown.ar（本体）と slowdown_core.ar（操作ごとの関数）を %TEMP% の作業場所へ写す。
#     コンパイル済みの作業場所では、測る exe 自身で slowdown_core を --compile する（リポジトリの直下で走らせる:
#     ar_config.json の clang を引くため）
#   * 非コンパイル → コンパイル済み（→ -B のコンパイル済み）の順に交互に走らせ、1 周目は捨てる。中央値を出す
#   * 出力: 操作ごとの VM・コンパイル済み・差（ns）。-B を付けると B のコンパイル済みと、その差も出す
#   * 1 回の実行は -TimeoutSec で打ち切り、プロセスの木ごと止めて ABNORMAL TERMINATION を出す
#
# ⚠ 数 ns の差はノイズの幅を同じセッションで測ってから判断する（`-B` に -Exe と同じ exe を渡すと対照になる）。

param(
    [string]$Exe = '',
    [string]$B = '',
    [int]$Reps = 5,
    [int]$Scale = 5,
    [int]$TimeoutSec = 300,
    [string]$WorkDir = ''
)

$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
$src = Join-Path $repo 'examples\bench\speed_modes'
if ($Exe -eq '') { $Exe = Join-Path $repo 'target\release\arrow.exe' }
if ($WorkDir -eq '') { $WorkDir = Join-Path $env:TEMP 'arrow_native_slowdown' }
foreach ($e in @($Exe, $B)) {
    if ($e -ne '' -and -not (Test-Path $e)) { throw "not found: $e" }
}

# 列: (名前, exe, コンパイルするか)
$cols = [ordered]@{ vm = @($Exe, $false); native = @($Exe, $true) }
if ($B -ne '') { $cols['native_b'] = @($B, $true) }

foreach ($c in $cols.Keys) {
    $dir = Join-Path $WorkDir $c
    if (Test-Path $dir) { Remove-Item -Recurse -Force $dir }
    New-Item -ItemType Directory -Force $dir | Out-Null
    foreach ($f in 'slowdown.ar', 'slowdown_core.ar') { Copy-Item (Join-Path $src $f) $dir }
    if ($cols[$c][1]) {
        Push-Location $repo
        $prev = $ErrorActionPreference
        $ErrorActionPreference = 'Continue'   # exe は進み具合を stderr に書く
        try {
            $out = & $cols[$c][0] --compile (Join-Path $dir 'slowdown_core.ar') 2>&1 | Out-String
        } finally {
            $ErrorActionPreference = $prev
            Pop-Location
        }
        if ($out -notmatch 'embedded') { throw "COMPILE-NO-DLL ${c}:`n$out" }
        foreach ($line in ($out -split "`r?`n")) { if ($line -match 'stays interpreted') { Write-Host "INTERP  $c  $line" } }
    }
}

function Invoke-Once([string]$exe, [string]$dir) {
    $psi = New-Object System.Diagnostics.ProcessStartInfo
    $psi.FileName = $exe; $psi.Arguments = 'slowdown.ar'; $psi.WorkingDirectory = $dir
    $psi.UseShellExecute = $false; $psi.RedirectStandardOutput = $true; $psi.RedirectStandardError = $true
    $psi.CreateNoWindow = $true
    $psi.EnvironmentVariables['AR_BENCH_SCALE'] = "$Scale"
    $p = [Diagnostics.Process]::Start($psi)
    $o = $p.StandardOutput.ReadToEndAsync(); $e = $p.StandardError.ReadToEndAsync()
    if (-not $p.WaitForExit($TimeoutSec * 1000)) {
        & taskkill /T /F /PID $p.Id | Out-Null
        Write-Host "ABNORMAL TERMINATION: $dir\slowdown.ar exceeded $TimeoutSec s (killed)" -ForegroundColor Red
        exit 1
    }
    $p.WaitForExit()
    if ($p.ExitCode -ne 0) { Write-Host "FAIL $dir exit=$($p.ExitCode)`n$($e.Result)" -ForegroundColor Red; exit 1 }
    $m = [ordered]@{}
    foreach ($line in ($o.Result -split "`r?`n")) {
        if ($line -match '^MICRO\s+(\S+)\s+(\S+)$') { $m[$Matches[1]] = [double]$Matches[2] }
    }
    return $m
}

function Median($xs) {
    $s = @($xs | Sort-Object); $n = $s.Count
    if ($n % 2) { return $s[($n - 1) / 2] }
    return ($s[$n / 2 - 1] + $s[$n / 2]) / 2
}

$acc = @{}
for ($r = 0; $r -le $Reps; $r++) {
    foreach ($c in $cols.Keys) {
        $m = Invoke-Once $cols[$c][0] (Join-Path $WorkDir $c)
        if ($r -eq 0) { continue }
        foreach ($k in $m.Keys) {
            if (-not $acc.ContainsKey("$c/$k")) { $acc["$c/$k"] = New-Object System.Collections.ArrayList }
            [void]$acc["$c/$k"].Add($m[$k])
        }
    }
    Write-Host ("round {0}/{1} done" -f $r, $Reps)
}

Write-Host ''
Write-Host "exe: $Exe"
if ($B -ne '') { Write-Host "B:   $B" }
$head = '{0,-20}{1,10}{2,10}{3,10}' -f 'metric (ns)', 'vm', 'native', 'diff'
if ($B -ne '') { $head += '{0,12}{1,10}' -f 'native_b', 'diff_b' }
Write-Host $head
$metrics = @($acc.Keys | Where-Object { $_ -like 'vm/*' } | ForEach-Object { $_.Substring(3) } | Sort-Object)
foreach ($k in $metrics) {
    $v = Median $acc["vm/$k"]
    $n = Median $acc["native/$k"]
    $line = '{0,-20}{1,10:N1}{2,10:N1}{3,10:N1}' -f $k, $v, $n, ($n - $v)
    if ($B -ne '') {
        $nb = Median $acc["native_b/$k"]
        $line += '{0,12:N1}{1,10:N1}' -f $nb, ($nb - $v)
    }
    Write-Host $line
}
Write-Host 'NATIVE-SLOWDOWN-DONE'
