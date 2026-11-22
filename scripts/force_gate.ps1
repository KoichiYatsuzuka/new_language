# force_gate.ps1 — #3（強制バイトコード）へ進めるかを判定するゲート（#25）。
#
# 全例題を `--vm=force` で実行し、**VM に載せられなかった箇所**（`VmForceError`）を列挙する。
# ここが 0 件になったら「例題の範囲では」フォールバックを撤去できる、という意味になる。
#
# ⚠ `AR_TW_STATS`（tw_stats.ps1）との役割分担:
#   - こちら      … **止めて判定する**ゲート。0 か 0 でないか。
#   - AR_TW_STATS … **数えるだけ**の計測。何がどこに何件あるか（潰す作業はこちらを見る）。
#   件数を数える目的にこのスクリプトを使わないこと（最初の 1 件で止まるため実数は分からない）。
#
# ⚠ 定義文（`fn`/`class`/`import` 等）は**設計上インタプリタが実行する**ので対象外
#   （制御フローも TLS も持たない。判断の根拠は #10-d・実装ログ）。
#
# ⚠ `_error` 例題は元々エラーで終わる。判定は**終了コードではなく `VmForceError` の有無**で行う。
#
# ── タイムアウト例題の扱い（#29）────────────────────────────────────────────
# GUI・対話・長時間ベンチの例題は時間内に終わらない。以前はそれを**黙って捨てて**いたので
# 「0 件」が「全例題で確かめた」を意味していなかった（ゲートの穴）。今は 2 点を直してある:
#
#   1. **タイムアウトでも stderr を読んで `VmForceError` を探す**（以前は読まずに捨てていた）。
#   2. **kill する前に窓を閉じて正常終了させる**（WM_CLOSE を繰り返し送る）。GUI 例題は
#      これで最後まで走るので、終了処理のコードまで検査できる。
#   3. それでも終わらなければ**サンプル判定**として名前を出す（黙って捨てない）。
#
# この 3 点で 5 例題あった未判定は **0 件**になった（128 例題すべて完走）。
#
# サンプル判定が意味を持つ根拠: `VmForceError` は `make_internal_raised_error` の許可リストに
# 無いので **Arrow の `try/except` では捕まらず、必ずプロセスを終わらせる**。
# ⇒ **タイムアウト時点で生きている＝その時点までに force エラーは起きていない**。
# 足りないのは「その先で初めて実行される経路」だけで、GUI のメインループのように
# 毎フレーム同じ本体を回す形なら実質的に覆えている。
#
# 使い方: ./scripts/force_gate.ps1 [-Timeout 45] [-Grace 25]
#   -Timeout … 1 例題を待つ秒数（`flat_bench` が 24 秒かかるので既定 45）
#   -Grace   … タイムアウト後に「窓を閉じ続ける」秒数（ダイアログを順に出す例題があるので既定 25）
#   ./scripts/force_gate.ps1 -Root gate   # gate/ のテスト（禁止の形を含む）を対象にする
#
# ⚠ 実行は共通ランナー（gate_run.ps1）が行い、ここは保存した結果から判定するだけ（gate_speed_plan.md 3-3）。
#   窓を閉じる猶予（-Grace）もランナーが行う（-GraceSec）。結果は target/gate_results/ に残り、同じ exe・同じファイルなら
#   次は実行せずに読む（読んだことを表示する。-Fresh で実行し直す）。
#   -Exclude … 対象から外すファイル（リポジトリ直下からの相対パスに対する正規表現。例: 窓を出す例題を外して測る）
param([int]$Timeout = 45, [int]$Grace = 25, [switch]$SkipTestBuild, [string]$Root = '', [switch]$Fresh, [int]$Jobs = 0, [string]$Exclude = '')


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

$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot   # scripts/ の 1 つ上 = リポジトリ直下
$exe = Join-Path $repo 'target\release\arrow.exe'
if (-not (Test-Path $exe)) { throw "not built: $exe (run: cargo build --release)" }


. (Join-Path $PSScriptRoot 'gate_run.ps1')
. (Join-Path $PSScriptRoot 'gate_judge.ps1')

if ($Root) {
    $rootDir = (Resolve-Path $Root).Path
    $files = @(Get-GateTestFiles $rootDir)
} else {
    $rootDir = Join-Path $repo 'examples'
    $files = @(Get-ChildItem -Path $rootDir -Filter *.ar -Recurse | Where-Object { $_.FullName -notmatch '\\archived\\' })
}
if ($Exclude) { $files = @($files | Where-Object { $_.FullName.Substring($repo.Length + 1) -notmatch $Exclude }) }

# 作業ディレクトリは例題のディレクトリ（今までと同じ）
$res = Invoke-GateRun -Exe $exe -Root $rootDir -Files @($files | ForEach-Object { $_.FullName }) -WorkDir file `
    -TimeoutSec $Timeout -GraceSec $Grace -Jobs $Jobs -Fresh:$Fresh -TreeExclude '^(DxLib|archived)\\'
$hits = @(Get-GateForceHits $res $repo)
$closed = @($res | Where-Object { $_.Closed } | ForEach-Object { $_.File.Substring($repo.Length + 1) })    # タイムアウト後に**自分で終了した**＝終了処理まで確認済み
$sampled = @($res | Where-Object { $_.TimedOut } | ForEach-Object { $_.File.Substring($repo.Length + 1) })  # kill した＝その先の経路は未確認
$ran = @($res | Where-Object { -not $_.TimedOut }).Count   # 完走（窓を閉じて終わったものも含む）


Write-Host ""
Write-Host ("examples: {0} 完走（うち {1} 件はタイムアウト後に窓を閉じて終了） / {2} サンプル判定（kill）" -f $ran, $closed.Count, $sampled.Count)
# ⚠ PS5.1 は `if` を式として書けない（PS7 の機能）。色は先に変数へ入れる。
$color = 'Yellow'
if ($hits.Count -eq 0) { $color = 'Green' }
Write-Host ("FORCE-GATE: {0} example(s) still fall back" -f $hits.Count) -ForegroundColor $color
if ($hits.Count -gt 0) {
    $hits | Sort-Object File | Format-Table -AutoSize -Wrap
}
if ($closed.Count -gt 0) {
    Write-Host ""
    Write-Host "タイムアウト後に窓を閉じて正常終了させた例題（終了処理まで検査済み）:" -ForegroundColor DarkGray
    $closed | Sort-Object | ForEach-Object { Write-Host "  $_" }
}
if ($sampled.Count -gt 0) {
    Write-Host ""
    Write-Host "サンプル判定（kill した＝その先の経路は未確認）:" -ForegroundColor DarkYellow
    $sampled | Sort-Object | ForEach-Object { Write-Host "  $_" }
}
Write-Host "FORCE-GATE-DONE"
