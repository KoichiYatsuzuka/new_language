# repl_session.ps1 — 対話 REPL の回帰検知（#36）。
#
# REPL は #36 でバイトコード VM 経路になった。compare_vm_modes.ps1 は stdin を与えないので
# この経路を覆えず、compare_debug_modes.ps1 はデバッガ REPL（別物）を見ている。
# ⇒ 対話 REPL は**このスクリプトだけ**が検査している。
#
# 検査内容: gate/repl/ の <name>.in をそれぞれ流し、<name>.out と一致するかを確認する（ブロックを跨ぐ状態・
# ブロックごとの解決情報の受け渡し）。⚠ 以前は examples/repl/repl_session.in の 1 本だけを見ていた。4-9 で
# gate/repl/ に写し、ディレクトリの中の対を全部見るようにした（examples/repl も -Dir で指せる）。
#
# 期待値を更新するとき: repl_session.ps1 -Update
#   ⚠ 差分の中身を必ず目で見ること（黙って上書きすると検知力を失う）。
#
# ⚠ stdin は **cmd のリダイレクト**で与える。PS5.1 の Process.StandardInput は
#   UTF-8 BOM を先頭に書いてしまい、REPL が ParseError: unexpected token になる。
#   ⚠ 正確には `Process.Start` が `[Console]::InputEncoding` で StreamWriter を作り、
#     `AutoFlush=true` を立てた時点で preamble を書く。**`.BaseStream` へ BOM 無しで
#     書いても遅い**（BOM は既に入っている）。ここは cmd のネイティブリダイレクトなので
#     マネージド writer 自体が作られず、原理的に踏まない。
#   ⚠ **debug_session.ps1 は `.BaseStream` 方式で踏んでいた**（#49 で
#     `[Console]::InputEncoding` を preamble 無しに差し替えて解消）。あちらが cmd 方式に
#     できないのは stdout/stderr を分けて受ける必要があるため。

param([switch]$Update, [string]$Dir = '')

$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot   # scripts/ の 1 つ上 = リポジトリ直下
$exe = Join-Path $repo 'target/release/arrow.exe'
if (-not (Test-Path $exe)) { throw "not built: $exe (run: cargo build --release)" }

$dir = if ($Dir) { (Resolve-Path $Dir).Path } else { Join-Path $repo 'gate/repl' }
$inputs = @(Get-ChildItem -LiteralPath $dir -Filter *.in | Sort-Object Name)
if ($inputs.Count -eq 0) { throw "no .in files in $dir" }

$identical = 0; $differing = 0
foreach ($in in $inputs) {
    $inFile = $in.FullName
    $expected = [IO.Path]::ChangeExtension($inFile, '.out')

    # stdout と stderr を 1 本にまとめて受ける（REPL は値を stdout・エラーを stderr へ出す）。
    # ⚠ 起動バナーは除く。ANSI エスケープと非 ASCII を含み、`cmd` 経由の取り込みが
    #   コードページ依存で欠落するため（期待値を環境間で持ち運べなくなる）。
    $lines = cmd /c "`"$exe`" --repl < `"$inFile`" 2>&1" | Where-Object { $_ -notmatch 'Arrow REPL' }
    $actual = ($lines -join "`n").TrimEnd() + "`n"

    if ($Update) {
        [System.IO.File]::WriteAllText($expected, $actual)
        Write-Host "  updated: $($in.BaseName) ($($actual.Length) bytes)" -ForegroundColor Yellow
        continue
    }
    if (-not (Test-Path $expected)) {
        Write-Host "  MISSING expected: $($in.BaseName).out (run with -Update)" -ForegroundColor Yellow
        $differing++
        continue
    }
    $want = ([System.IO.File]::ReadAllText($expected) -replace "`r`n", "`n").TrimEnd() + "`n"
    if ($actual -eq $want) { $identical++; continue }
    $differing++
    Write-Host "  DIFFERING: $($in.BaseName)" -ForegroundColor Red
    $wl = $want -split "`n"
    $al = $actual -split "`n"
    for ($i = 0; $i -lt [Math]::Max($wl.Count, $al.Count); $i++) {
        $w = if ($i -lt $wl.Count) { $wl[$i] } else { '<missing>' }
        $a = if ($i -lt $al.Count) { $al[$i] } else { '<missing>' }
        if ($w -ne $a) { Write-Host ("    line {0}: want [{1}]  got [{2}]" -f ($i + 1), $w, $a) }
    }
}
if ($Update) {
    Write-Host "REPL-SESSION: expected output updated ($($inputs.Count) file(s))" -ForegroundColor Yellow
    exit 0
}
if ($differing -eq 0) {
    Write-Host "REPL-SESSION: identical ($identical file(s))" -ForegroundColor Green
} else {
    Write-Host "REPL-SESSION: DIFFERING ($differing of $($inputs.Count) file(s))" -ForegroundColor Red
    exit 1
}
