# copy_stats.ps1 — 値の複製が、どの入口で何回起き、どれだけの大きさを写したかを数える（copy_alloc_speed_plan.md 1-2）
#
#   ./scripts/copy_stats.ps1                       # 専用のビルドを作り、9 カテゴリの例題と速度の調査のベンチを回す
#   ./scripts/copy_stats.ps1 -SkipBuild            # 作ってある exe（target\copystats）をそのまま使う
#   ./scripts/copy_stats.ps1 -Scripts a.ar,b.ar    # 指定した .ar だけ（リポジトリの直下からの相対）
#   ./scripts/copy_stats.ps1 -Csv out.csv          # 例題ごと・入口ごとの数を CSV にも出す
#
# 出力:
#   1. 入口の種類ごとの合計（例題の全体）
#   2. ベンチ（examples/bench/speed_modes/bench.ar・非コンパイル / コンパイル済み）の入口ごとの数
#   3. 写した値の数（nodes）が多い例題の上位
#
# 列（src/interpreter/copy_stats.rs）:
#   calls  … 複製の入口に来た回数           copied … 作り直す値（インスタンス・コンテナ）が来た回数
#   nodes  … 作り直した値の数（確保の目安）  elems  … 写した要素・フィールドの数
#   alone1 / alone2 / alone3 … 入れ子まで含めて誰とも共有していなかった回数（いちばん外の参照が 1 / 2 / 3 個まで）。
#     入口が値を受け取った 1 個のほかに、呼ぶ側が写しを持つ入口がある（コンパイル済みコードはアリーナの欄が 1 個）。
#     ⇒ `alone` が `copied` に近い入口は、写さずに移せる（計画のフェーズ 2）
#   skipped … `copied` のうち、入口が共有していないと判定して写さずに済ませた回数（実際に写したのは copied - skipped）
#   入口: let_self（`let self` ← `mut` の受け手）・let_param（`let` 仮引数 ← `mut` の値）・store（格納）・
#         auto_init_store（合成した `__init__` の直接格納）・bind（宣言の束縛。共有していない値は実際には写さない）・
#         explicit（`x.copy()`）。`let_self_readonly` / `let_param_readonly` は、`let_self` / `let_param` のうち呼ばれた関数が
#         読むだけと判定できたもの（`src/readonly_fns.rs`・計画の 4-1。VM の速い束縛の道だけ・内訳として重ねて数える）
#
# 仕組み:
#   * フックは feature `copy_stats` を付けたビルドにしか無い（既定のビルドではコードごと消える）。このスクリプトが
#     `target\copystats` に release で作る（`target\release` は触らない）
#   * `AR_COPY_STATS=1` で走らせ、実行の終わりに stderr へ出る `CopyStats[入口] calls=.. ...` を読む
#   * 1 回の実行は -TimeoutSec で打ち切り、プロセスの木ごと止める（`TIMEOUT` と出して次へ進む）
#
# ⚠ 回数は静的な判定（計画のフェーズ 3）や呼ばれた側が読むだけか（フェーズ 4）を区別しない。入口の種類と大きさだけ。
# ⚠ `powershell -File` 経由だと -Scripts a,b が 1 要素に潰れるので -Command で呼ぶこと。

param(
    [string]$Exe = '',
    [switch]$SkipBuild,
    [string[]]$Scripts = @(),
    [switch]$NoBench,
    [int]$TimeoutSec = 60,
    [int]$BuildTimeoutSec = 3600,
    [int]$Top = 15,
    [string]$Csv = ''
)

$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
$kinds = @('let_self', 'let_self_readonly', 'let_param', 'let_param_readonly', 'store', 'auto_init_store', 'bind', 'explicit')
$fields = @('calls', 'copied', 'nodes', 'elems', 'alone1', 'alone2', 'alone3', 'skipped')

# 時間の上限つきで 1 つのプロセスを走らせる。超えたらプロセスの木ごと止めて $null を返す。
function Invoke-Bounded([string]$file, [string]$arguments, [string]$cwd, [int]$limitSec, [hashtable]$envVars) {
    $psi = New-Object System.Diagnostics.ProcessStartInfo
    $psi.FileName = $file; $psi.Arguments = $arguments; $psi.WorkingDirectory = $cwd
    $psi.UseShellExecute = $false; $psi.RedirectStandardOutput = $true; $psi.RedirectStandardError = $true
    $psi.CreateNoWindow = $true
    foreach ($k in $envVars.Keys) { $psi.EnvironmentVariables[$k] = [string]$envVars[$k] }
    $p = [Diagnostics.Process]::Start($psi)
    $o = $p.StandardOutput.ReadToEndAsync(); $e = $p.StandardError.ReadToEndAsync()
    if (-not $p.WaitForExit($limitSec * 1000)) {
        & taskkill /T /F /PID $p.Id | Out-Null
        return $null
    }
    $p.WaitForExit()
    # ⚠ 子プロセス（js-proc の node など）が長生きするとパイプが閉じない。待つのは 10 秒まで（scan_examples.ps1 と同じ）
    $err = if ($e.Wait(10000)) { $e.Result } else { '' }
    $out = if ($o.Wait(10000)) { $o.Result } else { '' }
    return @{ Code = $p.ExitCode; Out = $out; Err = $err }
}

if ($Exe -eq '') {
    $target = Join-Path $repo 'target\copystats'
    $Exe = Join-Path $target 'release\arrow.exe'
    if (-not $SkipBuild) {
        Write-Host 'cargo build --release --features copy_stats (target\copystats)' -ForegroundColor Cyan
        $r = Invoke-Bounded 'cargo' 'build --release --features copy_stats' $repo $BuildTimeoutSec @{ CARGO_TARGET_DIR = $target }
        if ($null -eq $r) { Write-Host "ABNORMAL TERMINATION: cargo build exceeded $BuildTimeoutSec s (killed)" -ForegroundColor Red; exit 1 }
        if ($r.Code -ne 0) { throw "build failed:`n$($r.Err)" }
    }
}
if (-not (Test-Path $Exe)) { throw "not found: $Exe (run without -SkipBuild)" }
$Exe = (Resolve-Path $Exe).Path
Write-Host "exe: $Exe"

# stderr の `CopyStats[入口] k=v ...` を { 入口 = { 列 = 数 } } にする。1 行も無ければ $null（フックの無い exe）。
function Read-Stats([string]$err) {
    $m = @{}
    foreach ($line in ($err -split "`r?`n")) {
        if ($line -match '^CopyStats\[(\w+)\]\s+(.*)$') {
            $row = @{}
            foreach ($kv in ($Matches[2] -split '\s+')) {
                $p = $kv -split '='
                if ($p.Count -eq 2) { $row[$p[0]] = [long]$p[1] }
            }
            $m[$Matches[1]] = $row
        }
    }
    if ($m.Count -eq 0) { return $null }
    return $m
}

function Run-One([string]$name, [string]$arguments, [string]$cwd, [hashtable]$extraEnv) {
    $envVars = @{ AR_COPY_STATS = '1' }
    foreach ($k in $extraEnv.Keys) { $envVars[$k] = $extraEnv[$k] }
    $r = Invoke-Bounded $Exe $arguments $cwd $TimeoutSec $envVars
    if ($null -eq $r) { Write-Host "TIMEOUT`t$name"; return $null }
    if ($r.Code -ne 0) { Write-Host "FAIL`t$name`texit=$($r.Code)"; return $null }
    $s = Read-Stats $r.Err
    if ($null -eq $s) { throw "no CopyStats lines from $name (the exe has no copy_stats feature?)" }
    return $s
}

function Show-Table([string]$title, $stats) {
    Write-Host ''
    Write-Host $title -ForegroundColor Green
    Write-Host ('{0,-19}{1,11}{2,11}{3,11}{4,11}{5,11}{6,11}{7,11}{8,11}{9,5}{10,5}{11,5}' -f `
            'entry', 'calls', 'copied', 'skipped', 'nodes', 'elems', 'alone1', 'alone2', 'alone3', 'a1%', 'a2%', 'a3%')
    foreach ($k in $kinds) {
        $row = $stats[$k]
        if ($null -eq $row) { continue }
        $c = [double]$row['copied']
        $pct = { param($n) if ($c -gt 0) { '{0:N0}' -f (100.0 * $n / $c) } else { '-' } }
        Write-Host ('{0,-19}{1,11:N0}{2,11:N0}{3,11:N0}{4,11:N0}{5,11:N0}{6,11:N0}{7,11:N0}{8,11:N0}{9,5}{10,5}{11,5}' -f `
                $k, $row['calls'], $row['copied'], $row['skipped'], $row['nodes'], $row['elems'], $row['alone1'], $row['alone2'], $row['alone3'],
            (& $pct $row['alone1']), (& $pct $row['alone2']), (& $pct $row['alone3']))
    }
}

# ── 例題 ──
if ($Scripts.Count -gt 0) {
    $files = $Scripts | ForEach-Object { Get-Item (Join-Path $repo $_) }
}
else {
    # scan_examples.ps1 と同じ集合（9 カテゴリ・エラーの例題と止まらない例題を除く）
    $skip = @(
        'debug_demo', 'async_bench', 'async_demo', 'spider_render', 'spider_solitaire',
        'rs_struct', 'flat_bench', 'flat_bench_interp', 'flat_bench_module',
        'cs_form_app', 'cs_proc_app', 'js_proc_test', 'js_proc_async_test', 'math_render',
        'importation'
    )
    $cats = @('basics', 'collections', 'classes', 'typing', 'exceptions', 'async', 'bench', 'apps', 'interop')
    $files = $cats | ForEach-Object { Get-ChildItem "$repo\examples\$_\*.ar" -ErrorAction SilentlyContinue } |
        Where-Object { $n = $_.BaseName; -not ($n -match '_error' -or $n -match '__errors' -or $skip -contains $n) } |
        Sort-Object FullName
}

$total = @{}
foreach ($k in $kinds) { $total[$k] = @{}; foreach ($f in $fields) { $total[$k][$f] = [long]0 } }
$perExample = @()
$csvRows = New-Object System.Collections.ArrayList
$ran = 0
foreach ($f in $files) {
    $rel = $f.FullName.Substring($repo.Length + 1)
    $s = Run-One $rel ('"' + $f.FullName + '"') $repo @{}
    if ($null -eq $s) { continue }
    $ran++
    $nodes = [long]0
    foreach ($k in $kinds) {
        if (-not $s.ContainsKey($k)) { continue }
        foreach ($fld in $fields) { $total[$k][$fld] += $s[$k][$fld] }
        $nodes += $s[$k]['nodes']
        [void]$csvRows.Add(('{0},{1},{2}' -f $rel, $k, (($fields | ForEach-Object { $s[$k][$_] }) -join ',')))
    }
    $perExample += [pscustomobject]@{ Name = $rel; Nodes = $nodes; Stats = $s }
}
Show-Table "== examples: $ran file(s), totals by entry" $total

# ── ベンチ（非コンパイル / コンパイル済み） ──
if (-not $NoBench -and $Scripts.Count -eq 0) {
    $src = Join-Path $repo 'examples\bench\speed_modes'
    foreach ($mode in 'interp', 'native') {
        $dir = Join-Path $env:TEMP "arrow_copy_stats\$mode"
        if (Test-Path $dir) { Remove-Item -Recurse -Force $dir }
        New-Item -ItemType Directory -Force $dir | Out-Null
        foreach ($n in 'bench.ar', 'sim_core.ar') { Copy-Item (Join-Path $src $n) $dir }
        if ($mode -eq 'native') {
            # リポジトリの直下で走らせる（ar_config.json の clang を引くため）
            $c = Invoke-Bounded $Exe ('--compile "' + (Join-Path $dir 'sim_core.ar') + '"') $repo 300 @{}
            if ($null -eq $c -or ($c.Out + $c.Err) -notmatch 'embedded') { throw "COMPILE-NO-DLL ($mode)" }
        }
        $s = Run-One "bench($mode)" 'bench.ar' $dir @{ AR_BENCH_SCALE = '1' }
        if ($null -eq $s) { continue }
        Show-Table "== bench.ar ($mode, AR_BENCH_SCALE=1)" $s
        foreach ($k in $kinds) {
            if ($s.ContainsKey($k)) { [void]$csvRows.Add(('bench({0}),{1},{2}' -f $mode, $k, (($fields | ForEach-Object { $s[$k][$_] }) -join ','))) }
        }
    }
}

# ── 写した値の数が多い例題 ──
Write-Host ''
Write-Host "== top $Top examples by nodes copied" -ForegroundColor Green
Write-Host ('{0,-52}{1,12}  {2}' -f 'example', 'nodes', 'nodes by entry')
foreach ($e in ($perExample | Sort-Object Nodes -Descending | Select-Object -First $Top)) {
    $parts = foreach ($k in $kinds) { if ($e.Stats.ContainsKey($k) -and $e.Stats[$k]['nodes'] -gt 0) { '{0}={1}' -f $k, $e.Stats[$k]['nodes'] } }
    Write-Host ('{0,-52}{1,12:N0}  {2}' -f $e.Name, $e.Nodes, ($parts -join ' '))
}

if ($Csv -ne '') {
    $lines = @('example,entry,' + ($fields -join ',')) + $csvRows
    [System.IO.File]::WriteAllLines($Csv, $lines, (New-Object System.Text.UTF8Encoding($false)))
    Write-Host "csv: $Csv"
}
Write-Host 'COPY-STATS-DONE'
