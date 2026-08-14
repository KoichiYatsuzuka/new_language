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
# 使い方: ./force_gate.ps1 [-Timeout 20]
param([int]$Timeout = 20)

$ErrorActionPreference = 'Stop'
$exe = Join-Path $PSScriptRoot 'target\release\arrow.exe'
if (-not (Test-Path $exe)) { throw "not built: $exe (run: cargo build --release)" }

$files = Get-ChildItem -Path (Join-Path $PSScriptRoot 'examples') -Filter *.ar -Recurse |
         Where-Object { $_.FullName -notmatch '\\archived\\' }

$hits = @()
$ran = 0; $timedout = 0

foreach ($f in $files) {
    $psi = New-Object System.Diagnostics.ProcessStartInfo
    $psi.FileName = $exe
    $psi.Arguments = "-src `"$($f.FullName)`" --vm=force"
    $psi.WorkingDirectory = $f.DirectoryName
    $psi.UseShellExecute = $false
    $psi.RedirectStandardOutput = $true
    $psi.RedirectStandardError = $true

    $p = New-Object System.Diagnostics.Process
    $p.StartInfo = $psi
    [void]$p.Start()
    $errTask = $p.StandardError.ReadToEndAsync()
    $outTask = $p.StandardOutput.ReadToEndAsync()
    if (-not $p.WaitForExit($Timeout * 1000)) {
        try { $p.Kill() } catch {}
        $timedout++
        continue
    }
    $stderr = $errTask.Result
    [void]$outTask.Result
    $ran++

    foreach ($line in ($stderr -split "`n")) {
        if ($line -match 'VmForceError: (.+)$') {
            # ANSI 色コードを除いて読みやすくする
            $msg = ($Matches[1] -replace "\x1b\[[0-9;]*m", '').Trim()
            $hits += [pscustomobject]@{
                File = $f.FullName.Substring($PSScriptRoot.Length + 1)
                What = $msg
            }
            break   # 最初の 1 件で止まるので 1 ファイル 1 件
        }
    }
}

Write-Host ""
Write-Host "examples run: $ran (timeout: $timedout)"
# ⚠ PS5.1 は `if` を式として書けない（PS7 の機能）。色は先に変数へ入れる。
$color = 'Yellow'
if ($hits.Count -eq 0) { $color = 'Green' }
Write-Host ("FORCE-GATE: {0} example(s) still fall back" -f $hits.Count) -ForegroundColor $color
if ($hits.Count -gt 0) {
    $hits | Sort-Object File | Format-Table -AutoSize -Wrap
}
Write-Host "FORCE-GATE-DONE"
