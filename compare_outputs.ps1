# compare_outputs.ps1 — 2 つの arrow.exe で**全例題の stdout / stderr / exit code が同一か**を見る。
#
# ⚠⚠ **[compare_bytecode.ps1](compare_bytecode.ps1) では足りない場合のためのゲート**（#63 で新設）。
#    バイトコード同一性が言えるのは「コンパイラを触っていない」ことだけで、
#    **ツリーウォーク側（`eval_*` / `exec_*`）だけを触った変更では自明に一致してしまう**。
#    #63（`eval_method_call_full` のレシーバ別切り出し）はまさにその形で、
#    バイトコードは 1 バイトも変わらないのに**実行時の挙動は全部そこを通る**。
#    ⇒ **解釈側を触ったら bytecode ではなくこちらで「挙動不変」を主張すること。**
#
# 使い方:
#   ./compare_outputs.ps1 -A <head-arrow.exe> -B <new-arrow.exe>
#   ./compare_outputs.ps1 -A x.exe -B x.exe        # 負の対照（必ず 100% 一致）
#   ./compare_outputs.ps1 -A x.exe -ShowDiff       # 差分の中身も表示
#
# ⚠⚠ **使う前に必ず同一 exe 同士で負の対照を取る。** #63 の初回は 13 例題が DIFFERS になり、
#    原因は ①`bench` の経過時間 ②オブジェクトアドレス（`0x…` / `id()`）の 2 種だった。
#    前者は分類ごと除外、後者は `Normalize-Volatile` で正規化してある。
# ⚠ async・GUI・対話も対象外（揺れる／窓が出る／終わらない）。
#
# ⚠⚠ **子の出力はパイプで受けない**（#58 で実際にハングさせた）。`import[js-proc]` 等は
#    **孫プロセスが生き残ってパイプの書き込み端を握る**ので `ReadToEndAsync` でも返らない。
#    `Start-Process -RedirectStandard*` でファイルへ落としてから読む（skill `vm-pitfalls` §4）。
#
# ⚠ このファイルは**日本語コメントを含むので UTF-8 BOM 付きで保存すること**。
param(
    [Parameter(Mandatory=$true)][string]$A,
    [string]$B = '',
    [int]$TimeoutSec = 60,
    [switch]$ShowDiff
)

$ErrorActionPreference = 'Continue'
$repo = $PSScriptRoot
if ([string]::IsNullOrEmpty($B)) { $B = Join-Path $repo 'target/release/arrow.exe' }

foreach ($exe in @($A, $B)) {
    if (-not (Test-Path $exe)) { Write-Host "NOT FOUND: $exe" -ForegroundColor Red; exit 2 }
}

# GUI・対話・外部プロセス常駐は対象外。
$skip = @(
    'debug_demo', 'spider_render', 'spider_solitaire',
    'cs_form_app', 'math_render', 'importation', 'event_handler',
    # ⚠ `interop/` に置かれているが中身はベンチ（METRIC の経過時間を出す）。
    #    ディレクトリ単位の除外では漏れるので名指しで外す（#63 の負の対照で判明）。
    'bench_ab_cdll'
)
# ⚠ **`bench` ディレクトリは丸ごと対象外**（#63 の負の対照で判明）。出力が経過時間そのものなので
#    **同一バイナリでも 100% 一致しない**。速度の A/B は [ab_bench.ps1](ab_bench.ps1) の担当。
# ⚠ `async` も対象外（スケジューリング依存で揺れる。#52 と同じ理由）。
$categoryDirs = @('basics','collections','classes','typing','exceptions','apps','interop')

$examples = $categoryDirs | ForEach-Object {
    Get-ChildItem (Join-Path $repo "examples/$_/*.ar") -ErrorAction SilentlyContinue
} | Where-Object { $skip -notcontains $_.BaseName } | Sort-Object FullName

$tmp = Join-Path ([System.IO.Path]::GetTempPath()) ("ar_out_" + [guid]::NewGuid().ToString('N'))
$null = New-Item -ItemType Directory -Path $tmp

# ⚠⚠ **実行ごとに変わる値を潰す**（#63 の負の対照で判明）。潰さないと同一バイナリでも
#    3 例題が DIFFERS になる（`<X object at 0x...>` の表示と `id()` の戻り値）。
#    ⚠ **除外ではなく正規化にすること** — `collection.ar` は set メソッドの主要カバレッジで、
#    除外すると #63 で切り出した `eval_set_method` を見る網が消える。
function Normalize-Volatile([string]$text) {
    # `<Index object at 0x220064d3ee0>` 等のヒープアドレス
    $t = $text -replace '0x[0-9a-fA-F]+', '0xADDR'
    # `id(a): 2664622947840`（10 進のアドレス）。10 桁以上の整数だけを潰す。
    $t = $t -replace '(?<![\d.])\d{10,}(?![\d.])', '<ADDR>'
    return $t
}

function Get-Output([string]$exe, [string]$file, [string]$tag) {
    $o = Join-Path $tmp "$tag.out"
    $e = Join-Path $tmp "$tag.err"
    $p = Start-Process -FilePath $exe -ArgumentList ('"' + $file + '"') `
        -WorkingDirectory $repo -NoNewWindow -PassThru `
        -RedirectStandardOutput $o -RedirectStandardError $e
    if (-not $p.WaitForExit($TimeoutSec * 1000)) {
        try { $p.Kill() } catch {}
        return $null
    }
    $so = ''; $se = ''
    if (Test-Path $o) { $so = (Get-Content $o -Raw -Encoding UTF8) }
    if (Test-Path $e) { $se = (Get-Content $e -Raw -Encoding UTF8) }
    if ($null -eq $so) { $so = '' }
    if ($null -eq $se) { $se = '' }
    $text = ("EXIT=$($p.ExitCode)`n--- STDOUT ---`n" + ($so -replace "`r`n", "`n") +
             "`n--- STDERR ---`n" + ($se -replace "`r`n", "`n"))
    return (Normalize-Volatile $text)
}

$same = 0; $diff = 0; $timeouts = 0
$rows = @()
$i = 0

foreach ($f in $examples) {
    $i++
    $oa = Get-Output $A $f.FullName "a$i"
    $ob = Get-Output $B $f.FullName "b$i"
    if (($null -eq $oa) -or ($null -eq $ob)) {
        $timeouts++
        $rows += ("{0,-46} TIMEOUT (not compared)" -f $f.Name)
        continue
    }
    if ($oa -eq $ob) {
        $same++
    } else {
        $diff++
        $rows += ("{0,-46} DIFFERS" -f $f.Name)
        if ($ShowDiff) {
            $rows += (Compare-Object ($oa -split "`n") ($ob -split "`n") |
                Select-Object -First 20 |
                ForEach-Object { "    {0} {1}" -f $_.SideIndicator, $_.InputObject })
        }
    }
}

try { Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue } catch {}

if ($rows.Count -gt 0) {
    Write-Host 'DIFFERING / NOT COMPARED:' -ForegroundColor Yellow
    $rows | ForEach-Object { Write-Host $_ }
    Write-Host ''
}
Write-Host ("outputs identical: {0} / {1}   differing: {2}   timeout: {3}" -f `
    $same, ($same + $diff), $diff, $timeouts)
if ($diff -gt 0) { exit 1 } else { exit 0 }
