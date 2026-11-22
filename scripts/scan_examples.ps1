# scan_examples.ps1 — 全例題を実行し、失敗したものだけを理由付きで列挙する。
#
# _archive/run_examples.ps1（退避済み）との違い:
#   - タイムアウト付き（GUI/外部プロセス例題でハングしない）
#   - System.Diagnostics.Process を直接使う（Start-Process -PassThru の ExitCode は当てにならない）
#   - 失敗した例題の stderr 末尾を 1 行に畳んで表示する
#
# 使い方: .\scripts\scan_examples.ps1             # gate/ のテスト（禁止の形 `_error` を除く・gate_speed_plan.md 5-3）
#   ./scripts/scan_examples.ps1 -Root examples   # 利用者向けの例題（9 カテゴリ）。リリースの確かめ（release.ps1・D-16）
#   ⚠ 1 つでも FAIL / TIMEOUT があれば exit 1（5-3 から。以前は一覧を出すだけで exit 0 だった）。
#
# ⚠ 実行は共通ランナー（gate_run.ps1）が行い、ここは保存した結果から判定するだけ（gate_speed_plan.md 3-3）。
#   結果は target/gate_results/ に残り、同じ exe・同じファイルなら次は実行せずに読む（読んだことを表示する。-Fresh で実行し直す）。
param([string]$Exe = '', [int]$TimeoutSec = 45, [switch]$SkipTestBuild, [string]$Root = '', [switch]$Fresh, [int]$Jobs = 0)


# ── 前段: `cargo test`（debug）がビルドできるか（`test_build_gate.ps1`）────────────
# ⚠⚠ **このゲートは `target/release` しか見ない**ので、`cfg(debug_assertions)` の
#   強制点（`vm/compiler/mod.rs` の `storage_operands`）に `Op` を足し忘れても
#   最後まで緑で通ってしまう。実際に 2 回踏んでいる（`15aa677` / `c06254f`+`8c61d07`）。
#   ⇒ **走り出す前に debug のテストビルドだけ通す**（キャッシュが効けば 1 秒未満）。
#   ⚠ 合否ではなく**ビルドの可否**だけを見る。テスト本体は `cargo test` を別に。
if (-not $SkipTestBuild) {
    & (Join-Path $PSScriptRoot 'test_build_gate.ps1')
    if ($LASTEXITCODE -ne 0) {
        Write-Host '⇒ 先に cargo test（debug）のビルドを直すこと。' -ForegroundColor Red
        exit 1
    }
}

$ErrorActionPreference = 'Continue'
$repo = Split-Path -Parent $PSScriptRoot   # scripts/ の 1 つ上 = リポジトリ直下
if ([string]::IsNullOrEmpty($Exe)) { $Exe = Join-Path $repo 'target\release\arrow.exe' }

. (Join-Path $PSScriptRoot 'gate_run.ps1')
. (Join-Path $PSScriptRoot 'gate_judge.ps1')
# 例題の 9 カテゴリと、走らせない例題の一覧は gate_judge.ps1 にある（check_examples.ps1 と共有・5-4）。
$skip = $script:GateExampleSkip
$categoryDirs = $script:GateExampleCategories

# 対象（gate_speed_plan.md 5-3）: 既定は gate/。`-Root examples` で今までの例題の走査（9 カテゴリ・D-16）。
if (-not $Root) { $Root = Join-Path $repo 'gate' }
$rootDir = (Resolve-Path $Root).Path.TrimEnd('\')
$examplesMode = ($rootDir -eq (Join-Path $repo 'examples'))

if (-not $examplesMode) {
    # gate/ の正常系（禁止の形は gate_expect.ps1 が照らす・3-4）。走らせるのは全部（ほかのゲートと保存を共有するため・
    # Invoke-GateSuite）で、ここで出すのは `_error` でないものの FAIL / TIMEOUT だけ。
    if ($PSBoundParameters.ContainsKey('TimeoutSec')) { Write-Host 'scan_examples: -TimeoutSec is ignored for gate/ (fixed 60s, shared with the other gates)' -ForegroundColor DarkGray }
    $all = Invoke-GateSuite -Exe $Exe -Root $rootDir -Jobs $Jobs -Fresh:$Fresh
    $res = @($all | Where-Object { [IO.Path]::GetFileNameWithoutExtension($_.File) -notmatch '_error$' } | Sort-Object { Split-Path -Leaf $_.File })
} else {
    $examples = @($categoryDirs | ForEach-Object { Get-ChildItem "$repo\examples\$_\*.ar" -ErrorAction SilentlyContinue } | Where-Object {
        $n = $_.BaseName
        -not ($n -match '_error' -or $n -match '__errors' -or $skip -contains $n)
    } | Sort-Object Name)
    # ⚠ examples/ の木の指紋から、実行のたびに書き換わる大きなもの・対象外のディレクトリを除く（速さのため）
    # ⚠ 作業ディレクトリは**リポジトリ直下**のまま（3-3 で古い実装と同じ判定になることを確かめた）。
    $res = Invoke-GateRun -Exe $Exe -Root $rootDir -Files @($examples | ForEach-Object { $_.FullName }) -WorkDir $repo `
        -TimeoutSec $TimeoutSec -Jobs $Jobs -Fresh:$Fresh -TreeExclude '^(DxLib|archived|practical_examples)\\'
}
$fails = Write-GateScanReport $res
Write-Host "SCAN-DONE"
if ($fails -gt 0) { exit 1 } else { exit 0 }
