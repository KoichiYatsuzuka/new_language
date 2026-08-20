<#
.SYNOPSIS
  コメント内の `識別子` が src/ に実在するかを検査するゲート（#51）。

.DESCRIPTION
  この系列で最も繰り返した事故は「**削除・改名した関数を指すコメントが残る**」こと。
  #33 で消した `exec_for_stmt` / `has_escape` / `vm_mode` などを指す記述が 61 箇所あり、
  そのうち 2 件は**実際に指示が矛盾**していた（`toplevel_visible_globals` を渡せ、と
  書いてある一方で main.rs は「それを渡すな」と書いていた）。コンパイラは何も言わない。

  やること: src/**/*.rs のコメント行から `` `name` `` を拾い、
  コード側（コメントを除いた本文）にその識別子が 1 度も現れなければ報告する。

  ⚠ **履歴として正しい言及は落とす**。「削除済み」「#33 で削除した」「旧 …」のように
  マーカー語を同じ行に持つものは、意図的な記録なので検出しない。
  ⇒ 消した関数に言及したいときは **その行にマーカー語を書くこと**。

.PARAMETER All
  履歴扱い・ホワイトリストで落としたものも含めて全部出す（棚卸し用）。

.EXAMPLE
  ./stale_doc_refs.ps1          # ゲート（違反があれば exit 1）
  ./stale_doc_refs.ps1 -All     # 落としたものも見る
#>
param([switch]$All)

$ErrorActionPreference = 'Stop'
$root = Join-Path $PSScriptRoot 'src'
if (-not (Test-Path $root)) { Write-Error "src/ が見つからない: $root"; exit 2 }

# 履歴マーカー: この語が同じ行にあれば「意図的に消えたものへ言及している」とみなす
$histWords = @('削除', '廃止', '撤去', '以前', 'かつて', '旧 ', '旧`', '移設', 'だった', 'していた')

# 外部成果物・命名パターン（Rust の識別子ではないので永久に「存在しない」）
# ⚠ `extend_` / `_inner` / `snake_case` は**接頭辞・接尾辞・命名規約**であって識別子ではない。
#    「`compile_fn` とその `_inner`」のように規約を説明する書き方は正当なので落とす（#57）。
$whitelist = @(
    'force_gate','compare_vm_modes','compare_python_impl','scan_examples','debug_session',
    'repl_session','tw_stats_files','ab_bench','ab_bench_modes','dump_native_ir','prof_dist',
    'bench_field_access','fn_call','closure_call','block_expr','method_call','field_access',
    'cb_call_fn','snake_case','stubgen','extend_','_inner','ar_init','ar_event_fire'
)

$files = Get-ChildItem -Path $root -Recurse -Filter *.rs -File
$code = New-Object 'System.Collections.Generic.HashSet[string]'
$comments = New-Object 'System.Collections.Generic.List[object]'

foreach ($f in $files) {
    $rel = $f.FullName.Substring($PSScriptRoot.Length + 1).Replace('\', '/')
    $n = 0
    foreach ($line in [System.IO.File]::ReadAllLines($f.FullName)) {
        $n++
        if ($line.TrimStart().StartsWith('//')) {
            $comments.Add([pscustomobject]@{ File = $rel; Line = $n; Text = $line })
        } else {
            # 行内コメントを落としてから識別子を集める
            $bare = [regex]::Replace($line, '//.*', '')
            foreach ($m in [regex]::Matches($bare, '\w+')) { [void]$code.Add($m.Value) }
        }
    }
}

$bad = New-Object 'System.Collections.Generic.List[object]'
$skipped = 0
foreach ($c in $comments) {
    $isHist = $false
    foreach ($w in $histWords) { if ($c.Text.Contains($w)) { $isHist = $true; break } }
    foreach ($m in [regex]::Matches($c.Text, '`([A-Za-z_][A-Za-z0-9_]*(?:::[A-Za-z0-9_]+)*)(?:\(\))?`')) {
        $id = ($m.Groups[1].Value -split '::')[-1]
        if ($id.Length -lt 4) { continue }
        if ($id -cnotmatch '^[a-z_][a-z0-9_]*$') { continue }   # 型名・定数は対象外
        if ($code.Contains($id)) { continue }
        if ($whitelist -contains $id) { $skipped++; if (-not $All) { continue } }
        elseif ($isHist) { $skipped++; if (-not $All) { continue } }
        $bad.Add([pscustomobject]@{ Id = $id; File = $c.File; Line = $c.Line; Hist = $isHist })
    }
}

if ($bad.Count -eq 0) {
    Write-Host "OK: src/ のコメントに実在しない識別子への参照はありません（履歴・外部参照 $skipped 件を除外）" -ForegroundColor Green
    exit 0
}

Write-Host "存在しない識別子への参照: $($bad.Count) 件" -ForegroundColor Yellow
foreach ($g in ($bad | Group-Object Id | Sort-Object Count -Descending)) {
    Write-Host ("  {0}  ({1})" -f $g.Name, $g.Count) -ForegroundColor Yellow
    foreach ($b in $g.Group) {
        $tag = if ($b.Hist) { ' [履歴扱い]' } else { '' }
        Write-Host ("      {0}:{1}{2}" -f $b.File, $b.Line, $tag)
    }
}
Write-Host ""
Write-Host "直し方: ①現在の名前へ書き換える ②消えたものに言及したいなら同じ行に" -ForegroundColor Cyan
Write-Host "        「削除済み」等のマーカー語を書く ③外部成果物ならスクリプト冒頭の" -ForegroundColor Cyan
Write-Host "        `$whitelist に足す" -ForegroundColor Cyan
if ($All) { exit 0 }
exit 1
