# gate_expect.ps1 — gate/ のテストを実行し、正常系は「exit 0 で診断が 1 件も無い」こと、禁止の形（`_error`）は
# 「期待した場所で、期待した段階・種類の誤りが出て、ほかの誤りが出ない」ことを照らす（gate_speed_plan.md D-14 / D-15 / 3-4）。
#
# 期待の書き方は gate/README.md の 3 節。照らす規則（gate_judge.ps1 の Test-GateResult）:
#   - 出た誤り（警告を含む・AR_DIAG_OUT）の集まりと期待の集まりが一致すれば合格。足りない＝禁止が通った、余分＝別の理由で
#     落ちた。どちらも不合格（EXPECT-MISMATCH）。期待があるのに exit 0 も不合格。
#   - 警告だけを期待するテスト（`_warning`）は exit 0 で最後まで走ること（警告は実行を止めない）。
#   - 期待が無いテストは exit 0 で診断 0 件なら合格。GateCheckFailed で止まれば CHECK-FAILED（確かめ落ち）、ほかは FAILED。
#   - `_error` / `_warning` の名前と期待が食い違う・読めない期待・.arc の陳腐化の警告も不合格。
#
# 使い方:
#   ./scripts/gate_expect.ps1                 # gate/ の全部
#   ./scripts/gate_expect.ps1 -Root <dir>     # 別のディレクトリ（正の対照を取るときなど）
#   ./scripts/gate_expect.ps1 -Fresh          # 保存した結果を使わずに実行し直す
# 最後の行は `GATE-EXPECT: <合格> passed, <不合格> failed`。1 つでも不合格なら exit 1。
# ⚠ このファイルは日本語コメントを含むので UTF-8 BOM 付きで保存すること。
param([string]$Exe = '', [string]$Root = '', [int]$TimeoutSec = 60, [int]$Jobs = 0, [switch]$Fresh)

$ErrorActionPreference = 'Continue'
$repo = Split-Path -Parent $PSScriptRoot
if (-not $Exe) { $Exe = Join-Path $repo 'target\release\arrow.exe' }
if (-not $Root) { $Root = Join-Path $repo 'gate' }
$rootDir = (Resolve-Path $Root).Path

. (Join-Path $PSScriptRoot 'gate_run.ps1')
. (Join-Path $PSScriptRoot 'gate_judge.ps1')

if ($PSBoundParameters.ContainsKey('TimeoutSec')) {
    # 時間切れを変えると保存のキーが変わる（ほかのゲートと結果を共有しない）
    $files = @(Get-GateTestFiles $rootDir | Sort-Object FullName)
    $res = Invoke-GateRun -Exe $Exe -Root $rootDir -Files @($files | ForEach-Object { $_.FullName }) -WorkDir file `
        -TimeoutSec $TimeoutSec -Jobs $Jobs -Fresh:$Fresh
} else {
    $res = Invoke-GateSuite -Exe $Exe -Root $rootDir -Jobs $Jobs -Fresh:$Fresh   # 5-3: ほかのゲートと同じ引数
}
$pass = 0; $fail = 0
foreach ($r in $res) {
    $v = Test-GateResult $r
    if ($v.Pass) { $pass++; continue }
    $fail++
    $color = 'Red'
    if ($v.Status -eq 'CHECK-FAILED') { $color = 'Yellow' }
    Write-Host ("{0,-16} {1}" -f $v.Status, $r.Rel) -ForegroundColor $color
    foreach ($x in $v.Reasons) { Write-Host "    $x" }
}
$color = 'Green'
if ($fail -gt 0) { $color = 'Red' }
Write-Host ("GATE-EXPECT: {0} passed, {1} failed" -f $pass, $fail) -ForegroundColor $color
if ($fail -gt 0) { exit 1 } else { exit 0 }
