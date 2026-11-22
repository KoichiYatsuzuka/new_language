# gates_every_time.ps1 — 言語を変えるたびに走らせるゲートをまとめた入口（gate_speed_plan.md 5-3）。
#
# 手順: test_build_gate（cargo test がビルドできるか）→ gate/ のテストを 1 回だけ走らせる（共通ランナー・並列）→
#       その結果から判定する。判定は次の 3 つ（どれも実行し直さない）:
#   1. 期待との照合（gate_expect と同じ・Test-GateResult）: 正常系は exit 0 で診断 0 件、禁止の形は期待した誤りで止まる
#   2. VM に載らない構文が無いか（force_gate と同じ・stderr の VmForceError）
#   3. 静的な誤りと警告の全部の種類に期待を持つテストがあるか（gate_kinds.ps1）
# 1 つでも落ちたら exit 1。最後の行は `GATES-EVERY-TIME: OK` か `GATES-EVERY-TIME: FAILED (…)`。
#
# - examples/ は対象にしない（利用者向けの見本・D-13 / D-16。リリースのときに scripts/release.ps1 が確かめる）。
# - compare_python_impl は凍結した（D-9）ので入れていない。
# - 対話（repl_session / debug_session）と A/B 系は、触ったときに別に走らせる（CLAUDE.md の表）。
# - 結果は target/gate_results/ に残る。同じ exe・同じ gate/ なら 2 回目は実行せずに読む（-Fresh で実行し直す）。
#   scan_examples / force_gate / gate_expect も同じ引数で走らせる（Invoke-GateSuite）ので、この入口の後に個別に
#   走らせても実行し直さない。
#
# 使い方:
#   ./scripts/gates_every_time.ps1
#   ./scripts/gates_every_time.ps1 -Fresh          # 保存した結果を使わない
#   ./scripts/gates_every_time.ps1 -SkipTestBuild  # cargo test のビルドを確かめない
# ⚠ このファイルは日本語コメントを含むので UTF-8 BOM 付きで保存すること。
param([switch]$SkipTestBuild, [switch]$Fresh, [int]$Jobs = 0)

$ErrorActionPreference = 'Continue'
$repo = Split-Path -Parent $PSScriptRoot
$exe = Join-Path $repo 'target\release\arrow.exe'
if (-not (Test-Path $exe)) { Write-Host "not built: $exe (run: cargo build --release)" -ForegroundColor Red; exit 1 }
. (Join-Path $PSScriptRoot 'gate_run.ps1')
. (Join-Path $PSScriptRoot 'gate_judge.ps1')

$total = [Diagnostics.Stopwatch]::StartNew()
$failed = @()
$times = [ordered]@{}

# ── 1. cargo test がビルドできるか ──────────────────────────────────────────
if (-not $SkipTestBuild) {
    $sw = [Diagnostics.Stopwatch]::StartNew()
    & (Join-Path $PSScriptRoot 'test_build_gate.ps1')
    $times['test_build_gate'] = $sw.Elapsed.TotalSeconds
    if ($LASTEXITCODE -ne 0) {
        Write-Host '⇒ 先に cargo test（debug）のビルドを直すこと。' -ForegroundColor Red
        Write-Host 'GATES-EVERY-TIME: FAILED (test_build_gate)' -ForegroundColor Red
        exit 1
    }
}

# ── 2. gate/ を 1 回走らせる ────────────────────────────────────────────────
$sw = [Diagnostics.Stopwatch]::StartNew()
$res = Invoke-GateSuite -Exe $exe -Root (Join-Path $repo 'gate') -Jobs $Jobs -Fresh:$Fresh
$times['run gate/'] = $sw.Elapsed.TotalSeconds

# ── 3. 判定 ────────────────────────────────────────────────────────────────
$sw = [Diagnostics.Stopwatch]::StartNew()
$pass = 0; $bad = 0
foreach ($r in ($res | Sort-Object Rel)) {
    $v = Test-GateResult $r
    if ($v.Pass) { $pass++; continue }
    $bad++
    $color = if ($v.Status -eq 'CHECK-FAILED') { 'Yellow' } else { 'Red' }
    Write-Host ("{0,-16} {1}" -f $v.Status, $r.Rel) -ForegroundColor $color
    foreach ($x in $v.Reasons) { Write-Host "    $x" }
}
$color = if ($bad -eq 0) { 'Green' } else { 'Red' }
Write-Host ("GATE-EXPECT: {0} passed, {1} failed" -f $pass, $bad) -ForegroundColor $color
if ($bad -gt 0) { $failed += "gate_expect $bad" }

$hits = @(Get-GateForceHits $res (Join-Path $repo 'gate'))
foreach ($h in $hits) { Write-Host ("VmForceError  {0}: {1}" -f $h.File, $h.What) -ForegroundColor Red }
$color = if ($hits.Count -eq 0) { 'Green' } else { 'Red' }
Write-Host ("FORCE-GATE: {0} file(s) still fall back" -f $hits.Count) -ForegroundColor $color
if ($hits.Count -gt 0) { $failed += "force_gate $($hits.Count)" }

# gate_kinds は Write-Host で出すので、情報ストリーム（6）を受けて「届かない種類」の一覧の行だけを省く
& (Join-Path $PSScriptRoot 'gate_kinds.ps1') 6>&1 | Where-Object { "$_" -notmatch '^UNREACHABLE' } | ForEach-Object { Write-Host "$_" }
if ($LASTEXITCODE -ne 0) { $failed += 'gate_kinds' }
$times['judge'] = $sw.Elapsed.TotalSeconds

# ── まとめ ──────────────────────────────────────────────────────────────────
Write-Host ''
foreach ($k in $times.Keys) { Write-Host ("  {0,-16} {1,6:N1} s" -f $k, $times[$k]) -ForegroundColor DarkGray }
Write-Host ("  {0,-16} {1,6:N1} s" -f 'total', $total.Elapsed.TotalSeconds) -ForegroundColor DarkGray
if ($failed.Count -eq 0) {
    Write-Host 'GATES-EVERY-TIME: OK' -ForegroundColor Green
    exit 0
}
Write-Host ("GATES-EVERY-TIME: FAILED ({0})" -f ($failed -join ', ')) -ForegroundColor Red
exit 1
