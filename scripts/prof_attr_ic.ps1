# prof_attr_ic.ps1 — 属性アクセス IC の当たり外れを全例題で集計する（Phase R3 の計測）
#
# ── なぜ要るか ──────────────────────────────────────────────────────────────
# [BYTECODE_VM_PLAN.md](../implementation_logs/BYTECODE_VM_PLAN.md) の R3 は
# 「真に多相な protocol 引数は R3 の**多相 IC** に倒す」と書いているが、実装は
# エントリ 1 個の**単相 IC**（[ast.rs](../src/ast.rs) の `AttrCache`）である。
# その差を埋めるべきかを決めるのに、**多相化で救えるミスが実際どれだけあるか**が要る。
#
# ── 何を数えるか ────────────────────────────────────────────────────────────
# ミスを 2 つに分ける。**多相化（N-way）で救えるのは poly だけ**で、
# 分けずに「ミス率」だけ見ても規模の判断ができない。
#   hit  … class_id 一致
#   cold … キャッシュが空（その呼び出し点の初回）→ 多相化では救えない
#   poly … 埋まっているが class_id 違い（呼び出し点が多相）→ **ここだけ救える**
#
# ⚠ Phase T より前に測ってはいけない。テンプレートクラスが実体化ごとに新しい class_id を
#   取っていたので、多相性ではなく**キャッシュ欠落**を測ってしまう。
#
# 使い方:
#   cargo build --release --features prof
#   .\scripts\prof_attr_ic.ps1                 # 全例題を集計
#   .\scripts\prof_attr_ic.ps1 -Filter class   # ファイル名部分一致で絞る
#   .\scripts\prof_attr_ic.ps1 -Top 20         # poly miss の多い順に上位を出す

param(
    [string]$Filter = '',
    [int]$Top = 15,
    [int]$TimeoutSec = 20
)

$ErrorActionPreference = 'Continue'
$repo = Split-Path -Parent $PSScriptRoot
$exe = Join-Path $repo 'target/release/arrow.exe'
if (-not (Test-Path $exe)) { throw "not built: $exe (run: cargo build --release --features prof)" }

# 対話入力・GUI・外部プロセス依存・極端に長いもの（他のゲートと同じ方針）
$skip = @(
    'debug_demo', 'async_bench', 'async_demo', 'spider_render', 'spider_solitaire',
    'cs_form_app', 'cs_proc_app', 'js_proc_test', 'js_proc_async_test', 'math_render',
    'langtons_ant', 'langtons_ant_profile', 'bench_ab_native'
)

$files = Get-ChildItem -Path (Join-Path $repo 'examples') -Recurse -Filter *.ar |
    Where-Object { $_.FullName -notmatch '\\archived\\' } |
    Where-Object { $skip -notcontains $_.BaseName } |
    Where-Object { $Filter -eq '' -or $_.Name -like "*$Filter*" } |
    Sort-Object FullName

$totHit = [long]0; $totCold = [long]0; $totPoly = [long]0
$rows = @()
$ran = 0; $skipped = 0

foreach ($f in $files) {
    $psi = New-Object System.Diagnostics.ProcessStartInfo
    $psi.FileName = $exe
    $psi.Arguments = '"' + $f.FullName + '"'
    $psi.WorkingDirectory = $f.DirectoryName
    $psi.RedirectStandardOutput = $true
    $psi.RedirectStandardError = $true
    $psi.UseShellExecute = $false
    $psi.CreateNoWindow = $true
    $psi.EnvironmentVariables['AR_PROF'] = '1'

    $p = [System.Diagnostics.Process]::Start($psi)
    # ⚠ stdout/stderr は**先に非同期で吸う**。溜めたまま WaitForExit するとパイプが
    #   詰まって相手が止まる（他のゲートスクリプトと同じ罠）。
    $outTask = $p.StandardOutput.ReadToEndAsync()
    $errTask = $p.StandardError.ReadToEndAsync()
    if (-not $p.WaitForExit($TimeoutSec * 1000)) {
        try { $p.Kill() } catch {}
        $skipped++
        continue
    }
    $err = $errTask.Result
    $null = $outTask.Result
    $ran++

    $h = 0; $c = 0; $y = 0
    if ($err -match 'hit\s+(\d+)')       { $h = [long]$Matches[1] }
    if ($err -match 'cold miss\s+(\d+)') { $c = [long]$Matches[1] }
    if ($err -match 'poly miss\s+(\d+)') { $y = [long]$Matches[1] }
    if (($h + $c + $y) -eq 0) { continue }

    $totHit += $h; $totCold += $c; $totPoly += $y
    $rows += [pscustomobject]@{ Name = $f.Name; Hit = $h; Cold = $c; Poly = $y }
}

$total = $totHit + $totCold + $totPoly
Write-Host ''
Write-Host 'prof_attr_ic -- 属性アクセス IC の当たり外れ（全例題）'
Write-Host '------------------------------------------------------------------------------'
Write-Host ("examples run : {0}   timeout/skipped : {1}" -f $ran, $skipped)
if ($total -eq 0) { Write-Host 'probes: 0（prof feature 付きでビルドしたか確認すること）'; exit 0 }
$pct = { param($n) if ($total -eq 0) { 0.0 } else { $n * 100.0 / $total } }
Write-Host ("probes       : {0}" -f $total)
Write-Host ("  hit        : {0,14}  ({1:N4}%)" -f $totHit,  (& $pct $totHit))
Write-Host ("  cold miss  : {0,14}  ({1:N4}%)   <- 多相化では救えない" -f $totCold, (& $pct $totCold))
Write-Host ("  poly miss  : {0,14}  ({1:N4}%)   <- 多相化で救えるのはここだけ" -f $totPoly, (& $pct $totPoly))
Write-Host ''
Write-Host ("poly miss の多い例題（上位 {0}）:" -f $Top)
$rows | Where-Object { $_.Poly -gt 0 } | Sort-Object Poly -Descending |
    Select-Object -First $Top |
    Format-Table @{L='example';E={$_.Name};W=44}, Hit, Cold, Poly -AutoSize
Write-Host 'PROF-ATTR-IC-DONE'
