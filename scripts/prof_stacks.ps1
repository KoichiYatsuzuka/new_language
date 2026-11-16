# prof_stacks.ps1 — 実行時間の内訳をスタックのサンプリングで測る（copy_alloc_speed_plan.md 1-1）
#
#   ./scripts/prof_stacks.ps1                                   # 速度の調査のベンチの 6 区間を、コンパイル済みと VM で
#   ./scripts/prof_stacks.ps1 -Modes native -Kernels k2,k3      # モード・区間を絞る
#   ./scripts/prof_stacks.ps1 -Script examples/apps/foo.ar      # 任意の .ar を測る（その場で走らせる）
#   ./scripts/prof_stacks.ps1 -Incl deep_copy_value,bind_copy   # 包括時間を見たい関数（名前の一部）
#   ./scripts/prof_stacks.ps1 -Exe <arrow.exe>                  # 測る exe を指定する（PDB が隣にあること）
#
# 何を出すか（scripts/stack_prof/analyze.py）:
#   * from generated code (inclusive) … 生成コードが呼んだコールバックごとの包括時間（`(generated code itself)` が生成コード自身）
#   * self time                       … 葉の関数ごとの時間
#   * inclusive (name contains)       … -Incl の関数がスタックにある標本の割合
#   * heap                            … OS のヒープと確保の入口が葉の標本の割合
#   * heap by owner                   … 上の標本を、確保・解放を出している Arrow の関数ごとに分けたもの（copy_alloc_speed_plan.md 6-1）
#
# 仕組み:
#   * プロファイラ（scripts/stack_prof/sampler・C#）は対象を起動し、約 1 ms ごとに全スレッドを止めて `dbghelp` でスタックを
#     歩く。管理者権限は要らない。折りたたみ形式（根;...;葉 回数）を <WorkDir>\out\<名前>.folded に書く
#   * -Exe を省くと、**行の表だけのデバッグ情報つき**の exe を `target\profdbg` に作って測る（`cargo build --release` に
#     `CARGO_PROFILE_RELEASE_DEBUG=line-tables-only`。`target\release` は触らない）
#   * 区間のドライバ（scripts/stack_prof/kernels/p_k*.ar）は `examples/bench/speed_modes/sim_core.ar` の 1 区間だけを、
#     ベンチと同じ状態で繰り返す（`AR_PROF_M` 倍）。作業場所（%TEMP%）へ写して走らせ、コンパイル済みのモードは
#     測る exe 自身で `sim_core` を `--compile` する
#   * 1 回の実行は -TimeoutSec で打ち切り、プロセスの木ごと止めて ABNORMAL TERMINATION を出す
#
# ⚠ **デバッグ情報なしの exe を -Exe に渡さない。** PDB に内部の関数の記号が無く、近くの関数の名前に化ける
#   （調査で `get_var` / `ar_write_handle` に化けた）。
# ⚠ 止めて歩く分だけ遅くなる（1 標本 約 2〜3 ms）。**経過時間は測らない**（速度は speed_modes.ps1）。見るのは割合。
# ⚠ `powershell -File` 経由だと -Kernels a,b が 1 要素に潰れるので -Command で呼ぶこと。

param(
    [string]$Exe = '',
    [string[]]$Modes = @('native', 'interp'),
    [string[]]$Kernels = @('k1', 'k2', 'k3', 'k4', 'k5', 'k6'),
    [string]$Script = '',
    [int]$M = 0,
    [int]$Top = 20,
    [string]$Incl = 'deep_copy_value,copy_value,bind_copy',
    [int]$TimeoutSec = 600,
    [int]$BuildTimeoutSec = 3600,
    [string]$WorkDir = ''
)

$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
$profDir = Join-Path $PSScriptRoot 'stack_prof'
if ($WorkDir -eq '') { $WorkDir = Join-Path $env:TEMP 'arrow_prof_stacks' }
$outDir = Join-Path $WorkDir 'out'
New-Item -ItemType Directory -Force $outDir | Out-Null

# 繰り返しの倍率（区間ごと・モードごと）。コンパイル済みで 1 回 約 8〜12 秒、VM で 約 10〜20 秒になる数
$defaultM = @{
    native = @{ k1 = 30; k2 = 20; k3 = 12; k4 = 8; k5 = 8; k6 = 15 }
    interp = @{ k1 = 8; k2 = 12; k3 = 10; k4 = 6; k5 = 6; k6 = 8 }
}

# 時間の上限つきで 1 つのプロセスを走らせる。出力はファイルへ（孫が握ったパイプで止まらないように）。
function Invoke-Bounded([string]$file, [string[]]$argv, [string]$cwd, [int]$limitSec, [hashtable]$envVars, [string]$what) {
    $psi = New-Object System.Diagnostics.ProcessStartInfo
    $psi.FileName = $file
    $psi.Arguments = ($argv | ForEach-Object { if ($_ -match '\s') { '"' + $_ + '"' } else { $_ } }) -join ' '
    $psi.WorkingDirectory = $cwd
    $psi.UseShellExecute = $false
    $psi.RedirectStandardOutput = $true; $psi.RedirectStandardError = $true
    $psi.CreateNoWindow = $true
    foreach ($k in $envVars.Keys) { $psi.EnvironmentVariables[$k] = [string]$envVars[$k] }
    $p = [Diagnostics.Process]::Start($psi)
    $o = $p.StandardOutput.ReadToEndAsync(); $e = $p.StandardError.ReadToEndAsync()
    if (-not $p.WaitForExit($limitSec * 1000)) {
        & taskkill /T /F /PID $p.Id | Out-Null
        Write-Host "ABNORMAL TERMINATION: $what exceeded $limitSec s (killed)" -ForegroundColor Red
        exit 1
    }
    $p.WaitForExit()
    return @{ Code = $p.ExitCode; Out = $o.Result; Err = $e.Result }
}

# ── プロファイラを作る（ソースを作業場所へ写してビルドする。リポジトリに obj/ を作らない） ──
$samplerSrc = Join-Path $WorkDir 'sampler_src'
$samplerOut = Join-Path $WorkDir 'sampler_out'
$samplerExe = Join-Path $samplerOut 'sampler.exe'
$srcFiles = Get-ChildItem (Join-Path $profDir 'sampler') -File
$stale = -not (Test-Path $samplerExe)
if (-not $stale) {
    $built = (Get-Item $samplerExe).LastWriteTimeUtc
    $stale = @($srcFiles | Where-Object { $_.LastWriteTimeUtc -gt $built }).Count -gt 0
}
if ($stale) {
    if (Test-Path $samplerSrc) { Remove-Item -Recurse -Force $samplerSrc }
    New-Item -ItemType Directory -Force $samplerSrc | Out-Null
    $srcFiles | ForEach-Object { Copy-Item $_.FullName $samplerSrc }
    Write-Host 'building the sampler (dotnet build)' -ForegroundColor Cyan
    $r = Invoke-Bounded 'dotnet' @('build', '-c', 'Release', '-o', $samplerOut, '--nologo', '-v', 'q') $samplerSrc 600 @{} 'dotnet build'
    if ($r.Code -ne 0 -or -not (Test-Path $samplerExe)) { throw "sampler build failed:`n$($r.Out)`n$($r.Err)" }
}

# ── 測る exe（省いたら、行の表だけのデバッグ情報つきで作る） ──
if ($Exe -eq '') {
    $target = Join-Path $repo 'target\profdbg'
    $Exe = Join-Path $target 'release\arrow.exe'
    Write-Host 'cargo build --release (debug = line-tables-only, target\profdbg)' -ForegroundColor Cyan
    $r = Invoke-Bounded 'cargo' @('build', '--release') $repo $BuildTimeoutSec `
        @{ CARGO_TARGET_DIR = $target; CARGO_PROFILE_RELEASE_DEBUG = 'line-tables-only' } 'cargo build'
    if ($r.Code -ne 0 -or -not (Test-Path $Exe)) { throw "build failed:`n$($r.Err)" }
}
if (-not (Test-Path $Exe)) { throw "not found: $Exe" }
$Exe = (Resolve-Path $Exe).Path
if (-not (Test-Path ([System.IO.Path]::ChangeExtension($Exe, '.pdb')))) {
    throw "no PDB next to $Exe (build with CARGO_PROFILE_RELEASE_DEBUG=line-tables-only, or omit -Exe)"
}
Write-Host "exe: $Exe"

# 1 回測って集計を出す
function Measure-One([string]$name, [string]$cwd, [string]$script, [int]$m) {
    $folded = Join-Path $outDir "$name.folded"
    $r = Invoke-Bounded $samplerExe @($folded, "$TimeoutSec", $Exe, $script) $cwd ($TimeoutSec + 60) @{ AR_PROF_M = $m } "sampler $name"
    if ($r.Code -eq 3) { Write-Host "ABNORMAL TERMINATION: $name exceeded $TimeoutSec s (killed)" -ForegroundColor Red; exit 1 }
    if ($r.Code -ne 0) { Write-Host "FAIL sampler $name exit=$($r.Code)`n$($r.Out)`n$($r.Err)" -ForegroundColor Red; exit 1 }
    if ($r.Out -notmatch 'exit=0') {
        Write-Host "FAIL $name (the program failed): $($r.Out.Trim())`n$(Get-Content "$folded.stdout" -Raw)" -ForegroundColor Red
        exit 1
    }
    Write-Host ''
    Write-Host "#### $name  ($($r.Out.Trim()))" -ForegroundColor Green
    $a = Invoke-Bounded 'python' @((Join-Path $profDir 'analyze.py'), $folded, '--top', "$Top", '--incl', $Incl, '--heap-owners', '1') $repo 120 @{ PYTHONIOENCODING = 'utf-8' } 'analyze.py'
    if ($a.Code -ne 0) { Write-Host "FAIL analyze.py`n$($a.Out)`n$($a.Err)" -ForegroundColor Red; exit 1 }
    Write-Host $a.Out
}

if ($Script -ne '') {
    # 任意の .ar: その場で走らせる（import を解決できるよう、ファイルのあるディレクトリで）
    $full = (Resolve-Path $Script).Path
    $mm = if ($M -gt 0) { $M } else { 1 }
    Measure-One ([System.IO.Path]::GetFileNameWithoutExtension($full)) (Split-Path $full -Parent) (Split-Path $full -Leaf) $mm
    Write-Host 'PROF-STACKS-DONE'
    exit 0
}

$simCore = Join-Path $repo 'examples\bench\speed_modes\sim_core.ar'
foreach ($mode in $Modes) {
    if (-not $defaultM.ContainsKey($mode)) { throw "unknown mode '$mode' (native / interp)" }
    $dir = Join-Path $WorkDir $mode
    if (Test-Path $dir) { Remove-Item -Recurse -Force $dir }
    New-Item -ItemType Directory -Force $dir | Out-Null
    Copy-Item $simCore $dir
    Get-ChildItem (Join-Path $profDir 'kernels\p_*.ar') | ForEach-Object { Copy-Item $_.FullName $dir }
    if ($mode -eq 'native') {
        # リポジトリの直下で走らせる（ar_config.json の clang を引くため）
        $c = Invoke-Bounded $Exe @('--compile', (Join-Path $dir 'sim_core.ar')) $repo 300 @{} '--compile sim_core'
        if (($c.Out + $c.Err) -notmatch 'embedded') { throw "COMPILE-NO-DLL:`n$($c.Out)`n$($c.Err)" }
    }
    foreach ($k in $Kernels) {
        if (-not $defaultM[$mode].ContainsKey($k)) { throw "unknown kernel '$k' (k1..k6)" }
        $mm = if ($M -gt 0) { $M } else { $defaultM[$mode][$k] }
        Measure-One "${mode}_$k" $dir "p_$k.ar" $mm
    }
}
Write-Host 'PROF-STACKS-DONE'
