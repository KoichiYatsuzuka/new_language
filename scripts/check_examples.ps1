# check_examples.ps1 — 利用者向けの例題（examples/）が壊れていないかを確かめる（gate_speed_plan.md D-16 / 5-4）。
#
# examples/ は「毎回」のゲートの対象ではない（ゲートは gate/ だけを見る・D-13）。見本が壊れたまま利用者に届かないように、
# リリースの手順（scripts/release.ps1）がこれを呼ぶ。いつでも手で走らせてよい。
#
# 確かめること（9 カテゴリ・直下だけ。走らせない例題は gate_judge.ps1 の $GateExampleSkip）:
#   - 正常の例題（名前の末尾が `_error` / `_errors` でないもの）は exit 0 で終わる
#   - 禁止の例題（名前の末尾が `_error` / `_errors`）は exit が 0 でない（どの誤りで止まるかは照らさない・D-16）
#   - どれも時間切れにならない
#   ⚠ 名前が `_error` で終わるのに正常に終わる例題（主題が例外を捕まえること）は下の $successNamedError に理由つきで
#     載せ、正常の側で確かめる。
# 1 つでも合わなければ exit 1。最後の行は `CHECK-EXAMPLES: <合った数> ok, <合わない数> failed[, <許した数> allowed]`。
#
# 使い方:
#   ./scripts/check_examples.ps1
#   ./scripts/check_examples.ps1 -Allow event_external_handler   # この環境で落ちると分かっている例題を許す（理由を添えて使う）
# ⚠ このファイルは日本語コメントを含むので UTF-8 BOM 付きで保存すること。
param([string]$Exe = '', [string[]]$Allow = @(), [int]$TimeoutSec = 45, [int]$Jobs = 0, [switch]$Fresh)

$ErrorActionPreference = 'Continue'
$repo = Split-Path -Parent $PSScriptRoot
# `powershell -File … -Allow a,b` は 1 つの文字列 'a,b' で渡る（release.ps1 から呼ぶとき）ので、カンマで分ける
$Allow = @($Allow | ForEach-Object { $_ -split ',' } | ForEach-Object { $_.Trim() } | Where-Object { $_ })
if (-not $Exe) { $Exe = Join-Path $repo 'target\release\arrow.exe' }
. (Join-Path $PSScriptRoot 'gate_run.ps1')
. (Join-Path $PSScriptRoot 'gate_judge.ps1')

# 名前が `_error` で終わるのに正常に終わる例題（理由つき）
$successNamedError = [ordered]@{
    'eof_error'            = 'ファイルの EOFError / BOFError を捕まえる例題（主題が例外の名前）'
    'py_int_foreign_error' = 'Python が投げた例外を Arrow で捕まえる例題（主題が例外の名前）'
}

$files = @($script:GateExampleCategories | ForEach-Object { Get-ChildItem "$repo\examples\$_\*.ar" -ErrorAction SilentlyContinue } |
    Where-Object { $script:GateExampleSkip -notcontains $_.BaseName } | Sort-Object FullName)
$res = Invoke-GateRun -Exe $Exe -Root (Join-Path $repo 'examples') -Files @($files | ForEach-Object { $_.FullName }) -WorkDir $repo `
    -TimeoutSec $TimeoutSec -Jobs $Jobs -Fresh:$Fresh -TreeExclude '^(DxLib|archived|practical_examples)\\'

$ok = 0; $bad = 0; $allowed = 0
foreach ($r in ($res | Sort-Object Rel)) {
    $name = [IO.Path]::GetFileNameWithoutExtension($r.File)
    $isError = ($name -match '_errors?$') -and -not $successNamedError.Contains($name)
    $why = $null
    if ($r.TimedOut) { $why = 'timed out' }
    elseif ($isError -and $r.Exit -eq 0) { $why = 'exit 0: an _error example must stop with an error' }
    elseif (-not $isError -and $r.Exit -ne 0) {
        $tail = ((Read-GateText $r.ErrPath) -split "`n" | Where-Object { $_ -match '\S' } | Select-Object -Last 2) -join ' | '
        $why = "exit $($r.Exit): $(ConvertTo-GateOneLine $tail)"
    }
    if (-not $why) { $ok++; continue }
    if ($Allow -contains $name) { $allowed++; Write-Host ("ALLOWED  {0}  ({1})" -f $r.Rel, $why) -ForegroundColor DarkYellow; continue }
    $bad++
    Write-Host ("FAILED   {0}  ({1})" -f $r.Rel, $why) -ForegroundColor Red
}
foreach ($a in $Allow) { if (-not ($files | Where-Object { $_.BaseName -eq $a })) { Write-Host "ALLOW    $a is not an example that runs here (typo?)" -ForegroundColor Red; $bad++ } }
$tail = if ($allowed -gt 0) { ", $allowed allowed" } else { '' }
$color = if ($bad -eq 0) { 'Green' } else { 'Red' }
Write-Host ("CHECK-EXAMPLES: {0} ok, {1} failed{2}" -f $ok, $bad, $tail) -ForegroundColor $color
if ($bad -gt 0) { exit 1 } else { exit 0 }
