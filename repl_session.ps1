# repl_session.ps1 — 対話 REPL の回帰検知（#36）。
#
# REPL は #36 でバイトコード VM 経路になった。compare_vm_modes.ps1 は stdin を与えないので
# この経路を覆えず、compare_debug_modes.ps1 はデバッガ REPL（別物）を見ている。
# ⇒ 対話 REPL は**このスクリプトだけ**が検査している。
#
# 検査内容: examples/repl/repl_session.in を流し、examples/repl/repl_session.out と
# 一致するかを確認する（ブロックを跨ぐ状態・ブロックごとの解決情報の受け渡し）。
#
# 期待値を更新するとき: repl_session.ps1 -Update
#   ⚠ 差分の中身を必ず目で見ること（黙って上書きすると検知力を失う）。
#
# ⚠ stdin は **cmd のリダイレクト**で与える。PS5.1 の Process.StandardInput は
#   UTF-8 BOM を先頭に書いてしまい、REPL が ParseError: unexpected token になる。

param([switch]$Update)

$ErrorActionPreference = 'Stop'
$repo = $PSScriptRoot
$exe = Join-Path $repo 'target/release/arrow.exe'
if (-not (Test-Path $exe)) { throw "not built: $exe (run: cargo build --release)" }

$inFile = Join-Path $repo 'examples/repl/repl_session.in'
$expected = Join-Path $repo 'examples/repl/repl_session.out'

# stdout と stderr を 1 本にまとめて受ける（REPL は値を stdout・エラーを stderr へ出す）。
# ⚠ 起動バナーは除く。ANSI エスケープと非 ASCII を含み、`cmd` 経由の取り込みが
#   コードページ依存で欠落するため（期待値を環境間で持ち運べなくなる）。
$lines = cmd /c "`"$exe`" --repl < `"$inFile`" 2>&1" | Where-Object { $_ -notmatch 'Arrow REPL' }
$actual = ($lines -join "`n").TrimEnd() + "`n"

if ($Update) {
    [System.IO.File]::WriteAllText($expected, $actual)
    Write-Host "REPL-SESSION: expected output updated ($($actual.Length) bytes)" -ForegroundColor Yellow
    exit 0
}

$want = ([System.IO.File]::ReadAllText($expected) -replace "`r`n", "`n").TrimEnd() + "`n"
if ($actual -eq $want) {
    Write-Host 'REPL-SESSION: identical' -ForegroundColor Green
} else {
    Write-Host 'REPL-SESSION: DIFFERING' -ForegroundColor Red
    $wl = $want -split "`n"
    $al = $actual -split "`n"
    for ($i = 0; $i -lt [Math]::Max($wl.Count, $al.Count); $i++) {
        $w = if ($i -lt $wl.Count) { $wl[$i] } else { '<missing>' }
        $a = if ($i -lt $al.Count) { $al[$i] } else { '<missing>' }
        if ($w -ne $a) { Write-Host ("  line {0}: want [{1}]  got [{2}]" -f ($i + 1), $w, $a) }
    }
    exit 1
}
