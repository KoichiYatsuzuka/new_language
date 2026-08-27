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
param([int]$Timeout = 45, [int]$Grace = 25)

$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot   # scripts/ の 1 つ上 = リポジトリ直下
$exe = Join-Path $repo 'target\release\arrow.exe'
if (-not (Test-Path $exe)) { throw "not built: $exe (run: cargo build --release)" }

$files = Get-ChildItem -Path (Join-Path $repo 'examples') -Filter *.ar -Recurse |
         Where-Object { $_.FullName -notmatch '\\archived\\' }

$hits = @()
$sampled = @()   # kill した＝その先の経路は未確認
$closed = @()    # タイムアウト後に**自分で終了した**（窓を閉じる要求つき）＝終了処理まで確認済み
$ran = 0

foreach ($f in $files) {
    $psi = New-Object System.Diagnostics.ProcessStartInfo
    $psi.FileName = $exe
    $psi.Arguments = "-src `"$($f.FullName)`""
    $psi.WorkingDirectory = $f.DirectoryName
    $psi.UseShellExecute = $false
    $psi.RedirectStandardOutput = $true
    $psi.RedirectStandardError = $true

    $p = New-Object System.Diagnostics.Process
    $p.StartInfo = $psi
    [void]$p.Start()
    $errTask = $p.StandardError.ReadToEndAsync()
    $outTask = $p.StandardOutput.ReadToEndAsync()

    $rel = $f.FullName.Substring($repo.Length + 1)
    $timedOut = $false
    if (-not $p.WaitForExit($Timeout * 1000)) {
        # ⚠ **いきなり kill しない**（#29）。GUI 例題は「窓が閉じる」と正常終了するように
        # 書かれている（DxLib は `ProcessMessage()` が非 0 を返す／pygame は QUIT イベント）。
        # WM_CLOSE を送れば**終了処理のコードまで実行される**ので、kill より検査範囲が広い。
        #
        # ⚠ **1 回では足りない**。ダイアログを順に出す例題（`cs_form_app`）は、閉じるたびに
        # 次の窓が出るので**閉じ続ける**必要がある。`MainWindowHandle` はキャッシュされるので
        # 毎回 `Refresh()` してから送る。
        $closedOk = $false
        $deadline = (Get-Date).AddSeconds($Grace)
        while (-not $closedOk -and (Get-Date) -lt $deadline) {
            try {
                $p.Refresh()
                [void]$p.CloseMainWindow()
            } catch {}
            $closedOk = $p.WaitForExit(1000)
        }
        if ($closedOk) {
            $closed += $rel
        } else {
            try { $p.Kill() } catch {}
            $timedOut = $true
            $sampled += $rel
        }
    }
    # ⚠ **タイムアウトでも必ず stderr を読む**（#29）。kill でパイプが閉じるので待てば返る。
    #    以前はここを読まずに `continue` していたため、force エラーが出ていても取り逃していた。
    [void]$errTask.Wait(5000)
    [void]$outTask.Wait(5000)
    # ⚠ PS5.1 は `if` を式として書けない。先に既定値を入れてから上書きする。
    $stderr = ''
    if ($errTask.IsCompleted) { $stderr = $errTask.Result }
    if (-not $timedOut) { $ran++ }   # 完走（窓を閉じて終わったものも含む）

    foreach ($line in ($stderr -split "`n")) {
        if ($line -match 'VmForceError: (.+)$') {
            # ANSI 色コードを除いて読みやすくする
            $msg = ($Matches[1] -replace "\x1b\[[0-9;]*m", '').Trim()
            $hits += [pscustomobject]@{
                File = $rel
                What = $msg
            }
            break   # 最初の 1 件で止まるので 1 ファイル 1 件
        }
    }
}

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
