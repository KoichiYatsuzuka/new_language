# speed_modes.ps1 — 3 つの実行モード（非コンパイル / コンパイル済み / C++ の DLL）の速度の調査のベンチ
# （native_object_access_plan.md 7-1。例題は examples/bench/speed_modes/）
#
#   ./scripts/speed_modes.ps1                                  # target/release の exe で 3 モードを測る
#   ./scripts/speed_modes.ps1 -Exe <a.exe> -B <b.exe>          # 2 つの exe をモードごとに A/B
#   ./scripts/speed_modes.ps1 -Modes native -Only k2_energy    # モード・区間を絞る
#   ./scripts/speed_modes.ps1 -Micro                           # 1 操作の単価（ns）を測る（micro.ar / micro_ffi_bench.ar）
#   ./scripts/speed_modes.ps1 -Exes p0=<a.exe>,p1=<b.exe>,...  # 3 つ以上の exe を交互に測り、中央値を横に並べる
#                                                              # （マイルストーンの比較・最後の列は最初の exe との比）
#   ./scripts/speed_milestones.ps1                             # 計画書のマイルストーンの commit を作って上を走らせる
#
# 区間（bench.ar の METRIC）: k1_step（数値のメソッド）・k2_energy（list[Body] の反復＋ let self のメソッドの連鎖）・
#   k3_record（list[float] への追加と集計）・k4_labels（文字列の連結・int → str）・k5_spawn（生成）・
#   k6_boundary（インタープリタからコンパイル済みのメソッドを呼ぶ境界）・process_wall（起動を含むプロセス全体）・
#   peak_mb（プロセスの最大のワーキングセット・MB）
#
# 仕組み:
#   * 例題を %TEMP% の作業場所へ exe ごと・モードごとに写して走らせる（リポジトリを汚さない）。コンパイル済みのモードは
#     **測る exe 自身で** sim_core / micro_core を --compile する（.arc は作った exe の codegen を埋め込むので、
#     片方の .arc を他方が読むと比較にならない）
#   * C++ のモードは sim_kernels.dll を MSVC で作る（ar_config.json の cpp.msvc）。作れなければそのモードを
#     `SKIPPED cpp: <理由>` と出して飛ばす
#   * モードと exe を交互に走らせ、1 周目は捨てる（新しいファイルの読み込み・Defender の検査）
#   * CHECK（区間の結果）がモード間・exe 間で一致するかを見る（速くなったのではなく計算をしていないだけ、を見逃さない）
#   * 1 回の実行は -TimeoutSec で打ち切り、プロセスの木ごと止めて ABNORMAL TERMINATION を出す
#
# ⚠ 数 % の差はノイズの幅を同じセッションで測ってから判断する: `-Exe X -B X`（同じ exe 同士）が対照になる
#   （vm-pitfalls §1）。
# ⚠ `powershell -File` 経由だと -Modes a,b が 1 要素に潰れるので -Command で呼ぶこと。

param(
    [string]$Exe = '',
    [string]$B = '',
    [string[]]$Exes = @(),
    [string[]]$Modes = @('interp', 'native', 'cpp'),
    [int]$Reps = 7,
    [int]$Scale = 5,
    [string]$Only = '',
    [switch]$Micro,
    [int]$TimeoutSec = 600,
    [string]$Csv = '',
    [string]$WorkDir = ''
)

$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot   # scripts/ の 1 つ上 = リポジトリ直下
$src = Join-Path $repo 'examples\bench\speed_modes'
if ($Exe -eq '') { $Exe = Join-Path $repo 'target\release\arrow.exe' }
if ($WorkDir -eq '') { $WorkDir = Join-Path $env:TEMP 'arrow_speed_modes' }
foreach ($e in @($Exe, $B)) {
    if ($e -ne '' -and -not (Test-Path $e)) { throw "not found: $e" }
}
# ⚠ PowerShell の変数名は大文字と小文字を区別しない: 表を `$exes` と書くと型つきの引数 `[string[]]$Exes` に変換される
$exeList = @($Exes)
$runExes = [ordered]@{ A = $Exe }
if ($B -ne '') { $runExes['B'] = $B }
# 3 つ以上（`ラベル=パス`）。ラベルは作業場所のディレクトリ名にもなるので英数字・`_`・`-` だけにする
if ($exeList.Count -gt 0) {
    if ($B -ne '') { throw '-Exes and -B cannot be used together' }
    $runExes = [ordered]@{}
    foreach ($item in $exeList) {
        $i = $item.IndexOf('=')
        if ($i -le 0) { throw "-Exes takes label=path: $item" }
        $label = $item.Substring(0, $i); $path = $item.Substring($i + 1)
        if ($label -notmatch '^[A-Za-z0-9_-]+$') { throw "label must be [A-Za-z0-9_-]: $label" }
        if (-not (Test-Path $path)) { throw "not found: $path" }
        if ($runExes.Contains($label)) { throw "duplicate label: $label" }
        $runExes[$label] = $path
    }
}

# ── C++ のカーネル（1 回だけ作り、各作業場所へ写す） ───────────────────────────────
function Build-Kernels {
    $dir = Join-Path $WorkDir 'kernels'
    if (Test-Path $dir) { Remove-Item -Recurse -Force $dir }
    New-Item -ItemType Directory -Force $dir | Out-Null
    $cfg = Get-Content (Join-Path $repo 'ar_config.json') -Raw -Encoding utf8 | ConvertFrom-Json
    if ($null -eq $cfg.cpp -or $null -eq $cfg.cpp.msvc -or -not (Test-Path $cfg.cpp.msvc)) {
        return 'ar_config.json has no usable cpp.msvc (vcvarsall.bat)'
    }
    foreach ($f in 'sim_kernels.cpp', 'sim_kernels.h', 'sim_kernels.def') { Copy-Item (Join-Path $src "cpp\$f") $dir }
    Copy-Item (Join-Path $repo 'bridge\ptr_protocol\ar_ptr_protocol.h') $dir
    $bat = @"
@echo off
call "$($cfg.cpp.msvc)" x64 >nul 2>nul
cd /d "$dir"
cl /nologo /utf-8 /O2 /MD /EHsc /LD sim_kernels.cpp /link /DEF:sim_kernels.def /NOIMPLIB /OPT:REF /OPT:ICF /OUT:sim_kernels.dll >build.log 2>&1
if errorlevel 1 exit /b 1
"@
    $batFile = Join-Path $dir 'build.bat'
    Set-Content -Path $batFile -Value $bat -Encoding ascii
    & cmd /c $batFile | Out-Null
    if ($LASTEXITCODE -ne 0 -or -not (Test-Path (Join-Path $dir 'sim_kernels.dll'))) {
        return "MSVC build failed (see $dir\build.log)"
    }
    return ''
}

# ── exe ごと・モードごとの作業場所 ─────────────────────────────────────────────────
function Prepare([string]$tag, [string]$exe, [string]$mode) {
    $dir = Join-Path $WorkDir "$tag\$mode"
    if (Test-Path $dir) { Remove-Item -Recurse -Force $dir }
    New-Item -ItemType Directory -Force $dir | Out-Null
    # ar_config.json は実行する例題の上の階層から探される（clang・MSVC の場所）
    Copy-Item (Join-Path $repo 'ar_config.json') (Join-Path $WorkDir $tag) -Force
    if ($mode -eq 'cpp') {
        Copy-Item (Join-Path $src 'bench.ar') $dir
        foreach ($f in 'sim_core.ar', 'micro_ffi.ar', 'micro_ffi_bench.ar') { Copy-Item (Join-Path $src "cpp\$f") $dir }
        foreach ($f in 'sim_kernels.dll', 'sim_kernels.h', 'ar_ptr_protocol.h') { Copy-Item (Join-Path $WorkDir "kernels\$f") $dir }
        return
    }
    foreach ($f in 'bench.ar', 'sim_core.ar', 'micro.ar', 'micro_core.ar') { Copy-Item (Join-Path $src $f) $dir }
    if ($mode -eq 'native') {
        Push-Location $repo
        # ⚠ exe は進み具合を stderr に書く。'Stop' のままだと PowerShell 5.1 がそれを例外にする
        $prevPref = $ErrorActionPreference
        $ErrorActionPreference = 'Continue'
        try {
            foreach ($m in 'sim_core', 'micro_core') {
                $out = & $exe --compile (Join-Path $dir "$m.ar") 2>&1 | Out-String
                if ($out -notmatch 'embedded') { throw "COMPILE-NO-DLL ${tag}/${m}:`n$out" }
            }
        } finally {
            $ErrorActionPreference = $prevPref
            Pop-Location
        }
    }
}

# ── 1 回の実行 ─────────────────────────────────────────────────────────────────────
function Invoke-Run([string]$exe, [string]$dir, [string]$script) {
    $psi = New-Object System.Diagnostics.ProcessStartInfo
    $psi.FileName = $exe
    $psi.Arguments = '-src "' + (Join-Path $dir $script) + '"'
    $psi.WorkingDirectory = $dir
    $psi.UseShellExecute = $false
    $psi.RedirectStandardOutput = $true
    $psi.RedirectStandardError = $true
    $psi.CreateNoWindow = $true
    $psi.EnvironmentVariables['AR_BENCH_SCALE'] = "$Scale"
    $psi.EnvironmentVariables['AR_BENCH_ONLY'] = $Only
    $sw = [Diagnostics.Stopwatch]::StartNew()
    $p = [Diagnostics.Process]::Start($psi)
    $o = $p.StandardOutput.ReadToEndAsync()
    $e = $p.StandardError.ReadToEndAsync()
    # 最大のメモリ（ワーキングセット）も見る（1-15: 一時値が呼び出しごとに捨てられず 1 GB を超えたのを、時間の表だけでは
    # 見落とした）。終わった後は読めないので、走っている間に読む
    $peak = 0
    while (-not $p.WaitForExit(100)) {
        try { $p.Refresh(); if ($p.PeakWorkingSet64 -gt $peak) { $peak = $p.PeakWorkingSet64 } } catch {}
        if ($sw.Elapsed.TotalSeconds -gt $TimeoutSec) {
            & taskkill /T /F /PID $p.Id | Out-Null
            Write-Host "ABNORMAL TERMINATION: $dir\$script exceeded $TimeoutSec s (killed)" -ForegroundColor Red
            return $null
        }
    }
    $p.WaitForExit()
    $sw.Stop()
    if ($p.ExitCode -ne 0) {
        Write-Host "FAIL $dir\$script exit=$($p.ExitCode)`n$($e.Result)" -ForegroundColor Red
        return $null
    }
    $m = [ordered]@{}
    $c = [ordered]@{}
    foreach ($line in ($o.Result -split "`r?`n")) {
        if ($line -match '^(METRIC|MICRO)\s+(\S+)\s+(\S+)$') { $m[$Matches[2]] = [double]$Matches[3] }
        elseif ($line -match '^CHECK\s+(\S+)\s+(\S+)$') { $c[$Matches[1]] = $Matches[2] }
    }
    if (-not $Micro) { $m['process_wall'] = $sw.Elapsed.TotalSeconds }
    $m['peak_mb'] = $peak / 1MB
    return [pscustomobject]@{ Metrics = $m; Checks = $c }
}

function Script-For([string]$mode) {
    if (-not $Micro) { return 'bench.ar' }
    if ($mode -eq 'cpp') { return 'micro_ffi_bench.ar' }
    return 'micro.ar'
}

function Median($xs) {
    $s = @($xs | Sort-Object); $n = $s.Count
    if ($n % 2) { return $s[($n - 1) / 2] }
    return ($s[$n / 2 - 1] + $s[$n / 2]) / 2
}

# ── 準備 ───────────────────────────────────────────────────────────────────────────
New-Item -ItemType Directory -Force $WorkDir | Out-Null
$runModes = @()
foreach ($mode in $Modes) {
    if ($mode -notin @('interp', 'native', 'cpp')) { throw "unknown mode: $mode (interp / native / cpp)" }
    if ($mode -eq 'cpp') {
        $why = Build-Kernels
        if ($why -ne '') { Write-Host "SKIPPED cpp: $why" -ForegroundColor Yellow; continue }
    }
    foreach ($tag in $runExes.Keys) { Prepare $tag $runExes[$tag] $mode }
    $runModes += $mode
}
if ($runModes.Count -eq 0) { throw 'no mode to run' }

# ── 計測（モードと exe を交互に・1 周目は捨てる） ───────────────────────────────────
$acc = @{}
$checks = @{}
for ($r = 0; $r -le $Reps; $r++) {
    foreach ($mode in $runModes) {
        foreach ($tag in $runExes.Keys) {
            $res = Invoke-Run $runExes[$tag] (Join-Path $WorkDir "$tag\$mode") (Script-For $mode)
            if ($null -eq $res) { continue }
            if ($r -eq 0) { continue }
            $key = "$tag/$mode"
            if (-not $acc.ContainsKey($key)) { $acc[$key] = [ordered]@{} }
            foreach ($k in $res.Metrics.Keys) {
                if (-not $acc[$key].Contains($k)) { $acc[$key][$k] = New-Object System.Collections.ArrayList }
                [void]$acc[$key][$k].Add($res.Metrics[$k])
            }
            $checks[$key] = $res.Checks
        }
    }
    Write-Host ("round {0}/{1} done" -f $r, $Reps)
}

# ── 表 ─────────────────────────────────────────────────────────────────────────────
$rows = @()
$unit = 's'
if ($Micro) { $unit = 'ns' }
$labels = @($runExes.Keys)
foreach ($mode in $runModes) {
    if ($labels.Count -gt 2) {
        # 3 つ以上: 中央値を exe ごとに横に並べ、最後に「最初の exe ÷ 最後の exe」（1 より大きいほど速くなった）
        Write-Host ''
        Write-Host ("== {0} ({1}, median)" -f $mode, $unit)
        foreach ($t in $labels) { Write-Host ("   {0} = {1}" -f $t, $runExes[$t]) }
        $head = '{0,-30}' -f 'metric'
        foreach ($t in $labels) { $head += '{0,12}' -f $t }
        Write-Host ($head + ('{0,18}' -f "$($labels[0])/$($labels[-1])"))
        $first = "$($labels[0])/$mode"
        if (-not $acc.ContainsKey($first)) { Write-Host "no result: $mode" -ForegroundColor Red; continue }
        foreach ($k in $acc[$first].Keys) {
            $line = '{0,-30}' -f $k
            $meds = @()
            foreach ($t in $labels) {
                $key = "$t/$mode"
                if ($acc.ContainsKey($key) -and $acc[$key].Contains($k)) {
                    $xs = $acc[$key][$k]
                    $med = Median $xs
                    $meds += $med
                    $rows += [pscustomobject]@{ exe = $t; mode = $mode; metric = $k; min = ($xs | Measure-Object -Minimum).Minimum; median = $med; max = ($xs | Measure-Object -Maximum).Maximum; n = $xs.Count }
                    $line += '{0,12:N4}' -f $med
                } else {
                    $meds += $null
                    $line += '{0,12}' -f '-'
                }
            }
            $ratio = '-'
            if ($null -ne $meds[0] -and $null -ne $meds[-1] -and $meds[-1] -gt 0) { $ratio = '{0:N3}x' -f ($meds[0] / $meds[-1]) }
            Write-Host ($line + ('{0,18}' -f $ratio))
        }
        continue
    }
    $keyA = "A/$mode"
    if (-not $acc.ContainsKey($keyA)) { Write-Host "no result: $mode" -ForegroundColor Red; continue }
    Write-Host ''
    if ($runExes.Contains('B')) {
        Write-Host ("== {0} ({1}): A = {2}  B = {3}" -f $mode, $unit, $runExes['A'], $runExes['B'])
        Write-Host ("{0,-30}{1,12}{2,12}{3,12}{4,12}{5,10}" -f 'metric', 'A.min', 'B.min', 'A.med', 'B.med', 'A/B')
    } else {
        Write-Host ("== {0} ({1}): {2}" -f $mode, $unit, $runExes['A'])
        Write-Host ("{0,-30}{1,12}{2,12}{3,12}" -f 'metric', 'min', 'median', 'max')
    }
    foreach ($k in $acc[$keyA].Keys) {
        $xa = $acc[$keyA][$k]
        $minA = ($xa | Measure-Object -Minimum).Minimum
        $medA = Median $xa
        $maxA = ($xa | Measure-Object -Maximum).Maximum
        $rows += [pscustomobject]@{ exe = 'A'; mode = $mode; metric = $k; min = $minA; median = $medA; max = $maxA; n = $xa.Count }
        if ($runExes.Contains('B') -and $acc.ContainsKey("B/$mode") -and $acc["B/$mode"].Contains($k)) {
            $xb = $acc["B/$mode"][$k]
            $minB = ($xb | Measure-Object -Minimum).Minimum
            $medB = Median $xb
            $rows += [pscustomobject]@{ exe = 'B'; mode = $mode; metric = $k; min = $minB; median = $medB; max = ($xb | Measure-Object -Maximum).Maximum; n = $xb.Count }
            $ratio = '-'
            if ($medB -gt 0) { $ratio = '{0:N3}x' -f ($medA / $medB) }
            Write-Host ("{0,-30}{1,12:N4}{2,12:N4}{3,12:N4}{4,12:N4}{5,10}" -f $k, $minA, $minB, $medA, $medB, $ratio)
        } else {
            Write-Host ("{0,-30}{1,12:N4}{2,12:N4}{3,12:N4}" -f $k, $minA, $medA, $maxA)
        }
    }
}

# ── CHECK の一致 ───────────────────────────────────────────────────────────────────
# 区間のベンチ（bench.ar）は全モードが同じ計算なのでモード間・exe 間で比べる。1 操作の単価（-Micro）はモードごとに
# 別のスクリプトなので、同じモードの exe 間だけで比べる。
$groups = @{}
foreach ($key in $checks.Keys) {
    $g = 'all'
    if ($Micro) { $g = $key.Split('/')[1] }
    if (-not $groups.ContainsKey($g)) { $groups[$g] = @() }
    $groups[$g] += $key
}
$bad = 0
foreach ($g in $groups.Keys) {
    $keys = @($groups[$g] | Sort-Object)
    $ref = $checks[$keys[0]]
    foreach ($key in $keys) {
        foreach ($k in $ref.Keys) {
            if ($checks[$key][$k] -ne $ref[$k]) {
                $bad++
                Write-Host "CHECK MISMATCH $k : $($keys[0])=$($ref[$k]) $key=$($checks[$key][$k])" -ForegroundColor Yellow
            }
        }
    }
}
Write-Host ''
if ($bad -eq 0) { Write-Host "CHECK: all runs agree ($($checks.Keys.Count) mode/exe pair(s))" -ForegroundColor Green }
if ($Csv -ne '') { $rows | Export-Csv -NoTypeInformation -Encoding UTF8 -Path $Csv; Write-Host "csv: $Csv" }
Write-Host 'SPEED-MODES-DONE'
