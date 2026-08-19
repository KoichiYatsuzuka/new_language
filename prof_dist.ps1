# prof_dist.ps1 — **非コンパイル（解釈実行）の実行時間分布**を実測する（`--features prof`）。
#
# 何を出すか（2 軸）:
#   軸1 段別   : startup / lex / parse / type_check / resolve / interp_init / exec / teardown
#                （`Instant` の直接計測。段は数回しか通らないので計測費用は無視できる）
#   軸2 op 別  : exec の中で **どの op に何 ms 居たか**（別スレッドからの統計サンプリング）
#                ⚠ **命令数ではなく時間**。命令数の内訳は速度の予測に使えない（#46）。
#
# 使い方:
#   powershell -Command "./prof_dist.ps1 -Build"                      # prof ビルドを作る
#   powershell -Command "./prof_dist.ps1 -Mode phases"                # 全例題の段別分布
#   powershell -Command "./prof_dist.ps1 -Mode ops -Scripts examples/bench/bench_ab_interp.ar"
#   powershell -Command "./prof_dist.ps1 -Mode ops -Csv dist.csv"
#
# ⚠ 注意（この計測で実際に踏んだ落とし穴）:
#   * **1 回目の実行はファイルのコールドリードを踏む**（startup が 0.1ms → 10ms に化ける）。
#     既定で 2 パス走らせて **2 パス目だけ**を採用する（`-Passes 1` で無効化）。
#   * `-Mode ops` はサンプラースレッドが 1 コアをスピンするので **プロセス全体の wall が伸びる**。
#     wall を見たいときは `-Mode phases` の値を使うこと（op 内訳と wall を混ぜない）。
#   * 子プロセスの stdout/stderr は必ず `ReadToEndAsync()` で同時に読む（逐次読みはデッドロック）。
#   * `powershell -File` 経由だと `-Scripts a,b,c` が 1 要素に潰れるので `-Command` で呼ぶ。

param(
    [switch]$Build,
    [ValidateSet('phases', 'ops')][string]$Mode = 'phases',
    [string[]]$Scripts = @(),
    [int]$Passes = 2,
    [int]$SampleUs = 20,
    [int]$TimeoutSec = 180,
    [string]$Exe = '',
    [string]$Csv = '',
    [int]$Top = 20
)

$ErrorActionPreference = 'Stop'
$repo = $PSScriptRoot
if ([string]::IsNullOrEmpty($Exe)) { $Exe = Join-Path $repo 'target\release\arrow.exe' }

if ($Build) {
    # ⚠ cargo は進捗を stderr に出すので、その間だけ Stop を落とす（tw_stats.ps1 と同じ手当て）。
    $ErrorActionPreference = 'Continue'
    Write-Host 'cargo build --release --features prof' -ForegroundColor Cyan
    cargo build --release --features prof
    $ErrorActionPreference = 'Stop'
    if (-not (Test-Path $Exe)) { throw "build failed: $Exe not found" }
    Write-Host "built: $Exe" -ForegroundColor Green
}

if (-not (Test-Path $Exe)) {
    throw "$Exe not found. Run: powershell -Command `"./prof_dist.ps1 -Build`""
}

# 対象スクリプト（未指定なら scan_examples.ps1 と同じ集合）
if ($Scripts.Count -eq 0) {
    $skip = @(
        'debug_demo', 'async_bench', 'async_demo', 'spider_render', 'spider_solitaire',
        'rs_struct', 'flat_bench', 'flat_bench_interp', 'flat_bench_module',
        'cs_form_app', 'cs_proc_app', 'js_proc_test', 'js_proc_async_test', 'math_render',
        'importation'
    )
    $cats = @('basics', 'collections', 'classes', 'typing', 'exceptions', 'async', 'bench', 'apps', 'interop')
    $files = $cats | ForEach-Object { Get-ChildItem "$repo\examples\$_\*.ar" -ErrorAction SilentlyContinue } |
        Where-Object { $n = $_.BaseName; -not ($n -match '_error' -or $n -match '__errors' -or $skip -contains $n) } |
        Sort-Object Name
    $targets = $files | ForEach-Object { $_.FullName }
}
else {
    $targets = $Scripts | ForEach-Object { (Resolve-Path (Join-Path $repo $_)).Path }
}

function Invoke-One([string]$script, [hashtable]$envVars) {
    $psi = New-Object System.Diagnostics.ProcessStartInfo
    $psi.FileName = (Resolve-Path $Exe).Path
    $psi.Arguments = '-src "' + $script + '"'
    $psi.WorkingDirectory = $repo
    $psi.UseShellExecute = $false
    $psi.RedirectStandardOutput = $true
    $psi.RedirectStandardError = $true
    $psi.CreateNoWindow = $true
    foreach ($k in $envVars.Keys) { $psi.EnvironmentVariables[$k] = $envVars[$k] }

    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    $p = [System.Diagnostics.Process]::Start($psi)
    # ⚠ 逐次 ReadToEnd() は子とデッドロックする。必ず両方を同時に非同期で読む。
    $o = $p.StandardOutput.ReadToEndAsync()
    $e = $p.StandardError.ReadToEndAsync()
    if (-not $p.WaitForExit($TimeoutSec * 1000)) {
        try { $p.Kill() } catch {}
        $p.Dispose()
        return [pscustomobject]@{ Ok = $false; Reason = "TIMEOUT(${TimeoutSec}s)" }
    }
    $sw.Stop()
    $code = $p.ExitCode
    $err = $e.Result
    $null = $o.Result
    $p.Dispose()
    if ($code -ne 0) {
        $line = ($err -split "`n" | Where-Object { $_ -match '\S' } | Select-Object -Last 1)
        return [pscustomobject]@{ Ok = $false; Reason = "EXIT=$code $(($line -replace '\s+',' ').Trim())" }
    }
    return [pscustomobject]@{ Ok = $true; Err = $err; WallMs = $sw.Elapsed.TotalMilliseconds }
}

$envVars = @{ 'AR_PROF' = $(if ($Mode -eq 'ops') { 'ops' } else { '1' }) }
if ($Mode -eq 'ops') { $envVars['AR_PROF_US'] = "$SampleUs" }

$phaseSum = [ordered]@{}
foreach ($n in @('startup', 'lex', 'parse', 'type_check', 'resolve', 'interp_init', 'exec', 'teardown')) { $phaseSum[$n] = 0.0 }
$opSum = @{}
$rows = @()
$fails = @()
$wallSum = 0.0

for ($pass = 1; $pass -le $Passes; $pass++) {
    $keep = ($pass -eq $Passes)   # 最終パスだけ採用（コールドリードを捨てる）
    foreach ($t in $targets) {
        $r = Invoke-One $t $envVars
        if (-not $r.Ok) { if ($keep) { $fails += , @($t, $r.Reason) }; continue }
        if (-not $keep) { continue }
        $ph = [ordered]@{}
        foreach ($m in [regex]::Matches($r.Err, '(?m)^PHASE\s+(\S+)\s+([0-9.]+)\s+ms')) {
            $ph[$m.Groups[1].Value] = [double]$m.Groups[2].Value
        }
        # ⚠ 列挙中に本体を書き換えると PS が InvalidOperationException を投げるのでキーを複製する。
        foreach ($k in @($phaseSum.Keys)) { if ($ph.Contains($k)) { $phaseSum[$k] += $ph[$k] } }
        foreach ($m in [regex]::Matches($r.Err, '(?m)^OP\s+(\S+)\s+(\d+)\s+([0-9.]+)%\s+([0-9.]+)\s+ms$')) {
            $n = $m.Groups[1].Value
            if (-not $opSum.ContainsKey($n)) { $opSum[$n] = 0.0 }
            $opSum[$n] += [double]$m.Groups[4].Value
        }
        $wallSum += $r.WallMs
        $rows += [pscustomobject]@{
            Script  = (Resolve-Path -Relative $t)
            WallMs  = [math]::Round($r.WallMs, 2)
            InMain  = $(if ($ph.Contains('sum')) { $ph['sum'] } else { 0 })
            ExecMs  = $(if ($ph.Contains('exec')) { [math]::Round($ph['exec'], 3) } else { 0 })
            ParseMs = $(if ($ph.Contains('parse')) { [math]::Round($ph['parse'], 3) } else { 0 })
        }
    }
    Write-Host "pass $pass/$Passes done" -ForegroundColor DarkGray
}

Write-Host ''
Write-Host "=== phases ($($rows.Count) scripts, mode=$Mode) ===" -ForegroundColor Cyan
$tot = ($phaseSum.Values | Measure-Object -Sum).Sum
foreach ($k in @($phaseSum.Keys)) {
    '{0,-14} {1,10:N1} ms  {2,6:N2}%' -f $k, $phaseSum[$k], $(if ($tot -gt 0) { 100 * $phaseSum[$k] / $tot } else { 0 }) | Write-Host
}
'{0,-14} {1,10:N1} ms' -f 'in_main', $tot | Write-Host
'{0,-14} {1,10:N1} ms  (process wall - in_main; プロセス生成・イメージロード・終了)' -f 'outside main', ($wallSum - $tot) | Write-Host

if ($opSum.Count -gt 0) {
    $ot = ($opSum.Values | Measure-Object -Sum).Sum
    Write-Host ''
    Write-Host "=== ops in exec (top $Top of $($opSum.Count)) ===" -ForegroundColor Cyan
    $cum = 0.0
    $opSum.GetEnumerator() | Sort-Object Value -Descending | Select-Object -First $Top | ForEach-Object {
        $pct = 100 * $_.Value / $ot
        $cum += $pct
        '{0,-24} {1,10:N1} ms  {2,6:N2}%  cum {3,6:N1}%' -f $_.Key, $_.Value, $pct, $cum | Write-Host
    }
}

if ($fails.Count -gt 0) {
    Write-Host ''
    Write-Host "=== failures ===" -ForegroundColor Yellow
    foreach ($f in $fails) { Write-Host ("{0}`t{1}" -f $f[0], $f[1]) }
}

if ($Csv) {
    $out = @()
    foreach ($k in @($phaseSum.Keys)) { $out += [pscustomobject]@{ kind = 'phase'; name = $k; ms = $phaseSum[$k] } }
    foreach ($k in @($opSum.Keys)) { $out += [pscustomobject]@{ kind = 'op'; name = $k; ms = $opSum[$k] } }
    foreach ($r in $rows) { $out += [pscustomobject]@{ kind = 'script'; name = $r.Script; ms = $r.ExecMs } }
    $out | Export-Csv -Path $Csv -NoTypeInformation -Encoding UTF8
    Write-Host ''
    Write-Host "csv: $Csv" -ForegroundColor Green
}
Write-Host ''
Write-Host 'PROF-DIST-DONE' -ForegroundColor Green
