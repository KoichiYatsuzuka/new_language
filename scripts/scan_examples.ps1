# scan_examples.ps1 — 全例題を実行し、失敗したものだけを理由付きで列挙する。
#
# _archive/run_examples.ps1（退避済み）との違い:
#   - タイムアウト付き（GUI/外部プロセス例題でハングしない）
#   - System.Diagnostics.Process を直接使う（Start-Process -PassThru の ExitCode は当てにならない）
#   - 失敗した例題の stderr 末尾を 1 行に畳んで表示する
#
# 使い方: .\scripts\scan_examples.ps1
#   ./scripts/scan_examples.ps1 -Root gate   # gate/ のテスト（禁止の形 `_error` を除く）を対象にする（作業ディレクトリはファイルのディレクトリ）
#
# ⚠ 実行は共通ランナー（gate_run.ps1）が行い、ここは保存した結果から判定するだけ（gate_speed_plan.md 3-3）。
#   結果は target/gate_results/ に残り、同じ exe・同じファイルなら次は実行せずに読む（読んだことを表示する。-Fresh で実行し直す）。
#   ⚠ examples/ を対象にするときの作業ディレクトリは**リポジトリ直下**のまま（3-3 で古い実装と同じ判定になることを確かめた）。
param([string]$Exe = '', [int]$TimeoutSec = 45, [switch]$SkipTestBuild, [string]$Root = '', [switch]$Fresh, [int]$Jobs = 1)


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

$skip = @(
    'debug_demo', 'async_bench', 'async_demo', 'spider_render', 'spider_solitaire',
    'rs_struct', 'flat_bench', 'flat_bench_interp', 'flat_bench_module',
    'cs_form_app', 'cs_proc_app', 'js_proc_test', 'js_proc_async_test', 'math_render',
    'importation'
)
# ⚠⚠ **`archived/` と `practical_examples/` は意図的に対象外**（タスク 9.3・利用者の決定）。
#    このゲートを含め `$categoryDirs` を使う 4 本
#    （scan_examples / compare_outputs / compare_bytecode / compare_python_impl）は
#    9 カテゴリだけを走査する。
#    ⚠ `force_gate.ps1` と `compare_wasm_frontend.ps1` は `examples/` を `-Recurse` で
#      舐めるので上記 2 つも**含む**が、どちらも「例題が成功するか」は見ていない
#      （バイトコード化の可否／2 実装の診断の一致）。⇒ **壊れていても素通りする。**
#    ⚠⚠ **新しい検査の影響を測るときは母集団をこの 9 カテゴリに限ること。**
#      タスク 7.5 の偽陽性計測で `archived/` の壊れた例題を「偽陽性」と数えかけた。
#    理由と現状は examples/archived/README.md ・ examples/practical_examples/README.md。
$categoryDirs = @('basics','collections','classes','typing','exceptions','async','bench','apps','interop')

. (Join-Path $PSScriptRoot 'gate_run.ps1')
. (Join-Path $PSScriptRoot 'gate_judge.ps1')

if ($Root) {
    # gate/ の正常系（禁止の形は gate_expect.ps1 が照らす・3-4）
    $rootDir = (Resolve-Path $Root).Path
    $examples = @(Get-GateTestFiles $rootDir | Where-Object { $_.BaseName -notmatch '_error$' } | Sort-Object Name)
    $workDir = 'file'
} else {
    $rootDir = Join-Path $repo 'examples'
    $examples = @($categoryDirs | ForEach-Object { Get-ChildItem "$repo\examples\$_\*.ar" -ErrorAction SilentlyContinue } | Where-Object {
        $n = $_.BaseName
        -not ($n -match '_error' -or $n -match '__errors' -or $skip -contains $n)
    } | Sort-Object Name)
    $workDir = $repo
}

# ⚠ examples/ の木の指紋から、実行のたびに書き換わる大きなもの・対象外のディレクトリを除く（速さのため）
$res = Invoke-GateRun -Exe $Exe -Root $rootDir -Files @($examples | ForEach-Object { $_.FullName }) -WorkDir $workDir `
    -TimeoutSec $TimeoutSec -Jobs $Jobs -Fresh:$Fresh -TreeExclude '^(DxLib|archived|practical_examples)\\'
$null = Write-GateScanReport $res
Write-Host "SCAN-DONE"
