# debug_session.ps1 — 対話デバッガのステッピングの回帰検知（#1 で新設・#33 で golden 化）。
#
# ── なぜ golden 比較なのか ──────────────────────────────────────────────────
# 前身の compare_debug_modes.ps1 は `--vm=off` と `--vm=auto` の transcript を比べていたが、
# #33 で `--vm` を廃止したので**同じものを 2 回実行するだけ**になった（空回りする検査は
# 無い検査より悪い）。⇒ 期待値ファイルとの比較へ切り替えた（repl_session.ps1 と同じ方式）。
#
# 検査内容: examples/debugger/<name>.ar と同名の <name>.in（デバッガへ流すコマンド列）を対にして
# 実行し、<name>.out と一致するかを見る。`.in` が無い .ar はスキップする。
#
# ⚠ compare_vm_modes.ps1 は stdin を与えないのでこの経路を覆えない。
#   ⇒ **デバッガのステッピングはこのスクリプトだけが検査している**。
#
# 期待値を更新するとき: debug_session.ps1 -Update
#   ⚠ 差分の中身を必ず目で見ること（黙って上書きすると検知力を失う）。
#   ⇒ #44 以降、`-Update` は**書き換える行を必ず表示**する（UPDATING/UNCHANGED/CREATING）。

param(
    [string]$Filter = '',
    [int]$TimeoutSec = 60,
    [switch]$Update
)

$ErrorActionPreference = 'Continue'

$repo = $PSScriptRoot
$exe = Join-Path $repo 'target/release/arrow.exe'
if (-not (Test-Path $exe)) { throw "not built: $exe (run: cargo build --release)" }

$dir = Join-Path $repo 'examples/debugger'
if (-not (Test-Path $dir)) { throw "not found: $dir" }

# 1 本走らせて stdout+stderr を返す（stdin に $inputFile の内容を流す）。
# ⚠ 出力は**非同期で同時に読む**（逐次 ReadToEnd はデッドロックする・#38）。
# ⚠⚠ stdin へ BOM を混ぜないこと（#49）。**`.BaseStream` へ BOM 無しで書くだけでは足りない。**
#   .NET Framework の `Process.Start` は `StandardInput` の `StreamWriter` を
#   `[Console]::InputEncoding` で作り、`AutoFlush = true` を立てた時点で
#   **preamble を子の stdin へ書いてしまう** — つまり `Start()` が返った時点で既に
#   BOM が入っており、こちらが `BaseStream` に書くのはその**後ろ**になる。
#   ⇒ 起動前に **preamble 無しのエンコーディングへ差し替える**のが唯一効く手当て。
#   症状: デバッガの 1 行目が `?<BOM>` に化けて**コマンドが 1 つずれる**（全 5 件が赤くなる）。
#   ⚠ 発火はコンソールのコードページ依存（`chcp 65001` の環境で `[Console]::InputEncoding`
#     が preamble 付き utf-8 になる）。**別のマシンでは緑のまま**なので、
#     このゲートは「黙って赤くなる」癖がある（#44 に続き 2 度目）。
#   ⚠ **repl_session.ps1 は同じ罠を別の手で避けている**（`cmd /c "exe < file"` の
#     ネイティブリダイレクト＝マネージド writer を一切作らない）。あちらが緑のままだったのは
#     そのため。こちらが同じ手を採れないのは **stdout と stderr を分けて受ける必要がある**
#     から（`--stderr--` 区切り）＋タイムアウト時に `Kill()` したいから。
# 2 つの transcript の食い違う行だけを並べて出す。
# 比較（want/got）と -Update（old/new）で**同じ実装**を使う（片方だけ直すとずれる）。
function Show-Diff([string]$left, [string]$right, [string]$leftLabel, [string]$rightLabel) {
    $ll = $left -split "`n"; $rl = $right -split "`n"
    for ($i = 0; $i -lt [Math]::Max($ll.Count, $rl.Count); $i++) {
        $l = if ($i -lt $ll.Count) { $ll[$i] } else { '<missing>' }
        $r = if ($i -lt $rl.Count) { $rl[$i] } else { '<missing>' }
        if ($l -ne $r) {
            Write-Host ("    line {0}: {1} [{2}]  {3} [{4}]" -f ($i + 1), $leftLabel, $l, $rightLabel, $r)
        }
    }
}

function Invoke-Debug([string]$script, [string]$inputFile) {
    # 子を起こす直前だけ preamble 無しにして、必ず元へ戻す（#49・上のコメント参照）。
    $savedInputEncoding = [Console]::InputEncoding
    try { [Console]::InputEncoding = New-Object System.Text.UTF8Encoding($false) } catch {}
    try {
    $psi = New-Object System.Diagnostics.ProcessStartInfo
    $psi.FileName = $exe
    $psi.Arguments = "-src `"$script`""
    $psi.WorkingDirectory = $repo
    $psi.UseShellExecute = $false
    $psi.RedirectStandardInput = $true
    $psi.RedirectStandardOutput = $true
    $psi.RedirectStandardError = $true
    $psi.CreateNoWindow = $true

    $proc = [System.Diagnostics.Process]::Start($psi)
    $outTask = $proc.StandardOutput.ReadToEndAsync()
    $errTask = $proc.StandardError.ReadToEndAsync()
    $enc = New-Object System.Text.UTF8Encoding($false)
    $sw = New-Object System.IO.StreamWriter($proc.StandardInput.BaseStream, $enc)
    $sw.Write([System.IO.File]::ReadAllText($inputFile))
    $sw.Flush()
    $sw.Close()

    if (-not $proc.WaitForExit($TimeoutSec * 1000)) {
        try { $proc.Kill() } catch {}
        return "<<TIMEOUT>>"
    }
    $text = "$($outTask.Result)`n--stderr--`n$($errTask.Result)"
    # 絶対パスは環境依存なので伏せる（期待値ファイルを持ち運べるように）
    $text = $text -replace [regex]::Escape($repo), '<REPO>'
    return ($text -replace "`r`n", "`n").TrimEnd()
    } finally {
        try { [Console]::InputEncoding = $savedInputEncoding } catch {}
    }
}

$scripts = Get-ChildItem "$dir/*.ar" | Where-Object {
    ($Filter -eq '') -or ($_.BaseName -like "*$Filter*")
} | Sort-Object Name

$identical = 0; $differing = 0; $skipped = 0; $updated = 0
$diffNames = @()

foreach ($s in $scripts) {
    $inFile = Join-Path $dir "$($s.BaseName).in"
    if (-not (Test-Path $inFile)) { $skipped++; continue }
    $outFile = Join-Path $dir "$($s.BaseName).out"

    $actual = Invoke-Debug $s.FullName $inFile

    if ($Update) {
        # ⚠⚠ **黙って上書きしない**（#44）。何をどう書き換えるのかを必ず出してから書く。
        # `6bf039c`（#33 partial）は「stdin の BOM 修正」と「修正前の golden」を
        # **同じコミット**に入れ、録り直し漏れに誰も気づけないまま 5 件が赤くなった
        # （しかも完了報告には「5 identical」と書かれていた）。差分を出していれば気づけた。
        if (-not (Test-Path $outFile)) {
            Write-Host "  CREATING: $($s.BaseName)" -ForegroundColor Yellow
            [System.IO.File]::WriteAllText($outFile, $actual)
            $updated++
            continue
        }
        $prev = ([System.IO.File]::ReadAllText($outFile) -replace "`r`n", "`n").TrimEnd()
        if ($prev -eq $actual) {
            Write-Host "  UNCHANGED: $($s.BaseName)" -ForegroundColor DarkGray
            continue
        }
        Write-Host "  UPDATING: $($s.BaseName)" -ForegroundColor Yellow
        Show-Diff $prev $actual 'old' 'new'
        [System.IO.File]::WriteAllText($outFile, $actual)
        $updated++
        continue
    }
    if (-not (Test-Path $outFile)) {
        Write-Host "  MISSING expected: $($s.BaseName).out (run with -Update)" -ForegroundColor Yellow
        $differing++; $diffNames += $s.BaseName
        continue
    }
    $want = ([System.IO.File]::ReadAllText($outFile) -replace "`r`n", "`n").TrimEnd()
    if ($actual -eq $want) { $identical++ }
    else {
        $differing++; $diffNames += $s.BaseName
        Write-Host "  DIFFERING: $($s.BaseName)" -ForegroundColor Red
        Show-Diff $want $actual 'want' 'got'
    }
}

Write-Host ''
if ($Update) {
    Write-Host ("DEBUG-SESSION: updated {0} expected file(s)" -f $updated) -ForegroundColor Yellow
    exit 0
}
Write-Host ("identical: {0}   differing: {1}   skipped (no .in): {2}" -f $identical, $differing, $skipped)
if ($differing -eq 0) {
    Write-Host 'DEBUG-SESSION: clean' -ForegroundColor Green
    exit 0
} else {
    Write-Host ('DEBUG-SESSION: FAILED - ' + ($diffNames -join ', ')) -ForegroundColor Red
    exit 1
}
