<#
.SYNOPSIS
  「型義務」ごとに型検査が走っているかを測る網の目チェック（静的型検査の見直し用）。

.DESCRIPTION
  この系列で繰り返した事故は「**型検査が必要な地点を数え上げる仕組みが無い**」こと。
  検査は AST の地点ごとに手書きされた `if !type_matches(..) { push_error }` の集合で、
  書き忘れた地点は**誰も気づかない**（コンパイラも他のゲートも何も言わない）。
  0-1〜0-12 / A-2〜A-4 の 16 件の穴はすべてこの形だった。

  ⇒ 「型義務」= 2 つの型が適合しなければならない地点を `scripts/type_obligations/*.ar`
     として 1 件 1 ファイルで持ち、**意図的に型エラーを 1 つだけ**仕込む。
     それを実行して次のどれになるかを測る:

     | 判定    | 意味 |
     |---------|------|
     | STATIC  | StaticTypeError が出た（= 静的検査がある。あるべき姿） |
     | RUNTIME | 実行が始まってから TypeError 等で落ちた（= 実行経路を踏まなければ見逃す） |
     | NONE    | 完走した（= **無検査**。黙って不整合な型が通った） |

  各検体の先頭に `# @baseline:` で現状を焼いてある。**baseline より弱くなったら退行**として
  exit 1。強くなったら（NONE→RUNTIME→STATIC）PROGRESS として報告し、baseline の更新を促す。

  ⚠ 検体は `scripts/` 配下に置く。`examples/` に置くと scan_examples / force_gate /
     compare_* が「失敗する例題」として拾ってしまう（意図的に落とす検体なので）。

  ⚠ **これはカバレッジではなく網の目の検査**。「例題が一度も書いていない構文」を探す
     syntax_cov.ps1 と対になる（あちらは構文、こちらは型義務）。

.PARAMETER Filter
  ID または義務の説明の部分一致で絞り込む。

.PARAMETER ShowAll
  baseline と一致した検体も 1 行ずつ出す（既定は集計と差分のみ）。

.PARAMETER UpdateBaseline
  測った結果で各検体の `# @baseline:` を書き換える。**強くなったときだけ使うこと。**

.EXAMPLE
  ./scripts/type_obligations.ps1                # 退行検査（差分があれば exit 1）
  ./scripts/type_obligations.ps1 -ShowAll       # 全件の表を見る
  ./scripts/type_obligations.ps1 -Filter O      # 演算子の義務だけ
  ./scripts/type_obligations.ps1 -UpdateBaseline
#>
param(
    [string]$Filter = '',
    [switch]$ShowAll,
    [switch]$UpdateBaseline,
    [int]$TimeoutSec = 25
)

$ErrorActionPreference = 'Continue'
$repo = Split-Path -Parent $PSScriptRoot
$exe = Join-Path $repo 'target/release/arrow.exe'
if (-not (Test-Path $exe)) { throw "not built: $exe (run: cargo build --release)" }

$dir = Join-Path $PSScriptRoot 'type_obligations'
if (-not (Test-Path $dir)) { throw "probe corpus not found: $dir" }

$probes = Get-ChildItem "$dir/*.ar" | Sort-Object Name

function Get-Verdict([string]$file) {
    $psi = New-Object System.Diagnostics.ProcessStartInfo
    $psi.FileName = $exe
    $psi.Arguments = '-src "' + $file + '"'
    $psi.WorkingDirectory = $repo
    $psi.UseShellExecute = $false
    $psi.RedirectStandardOutput = $true
    $psi.RedirectStandardError = $true
    $psi.CreateNoWindow = $true
    $proc = [System.Diagnostics.Process]::Start($psi)
    # ⚠ 逐次 ReadToEnd は子とデッドロックする（vm-pitfalls §4）。必ず両方を非同期で開始する。
    $o = $proc.StandardOutput.ReadToEndAsync()
    $e = $proc.StandardError.ReadToEndAsync()
    if (-not $proc.WaitForExit($TimeoutSec * 1000)) {
        try { $proc.Kill() } catch {}
        return @{ Verdict = 'TIMEOUT'; Msg = '' }
    }
    $all = $o.Result + $e.Result
    # ANSI 色を落とす（診断表は色付きで出る）
    $all = $all -replace "`e\[[0-9;]*m", '' -replace "$([char]27)\[[0-9;]*m", ''
    $verdict =
        if ($all -match 'StaticTypeError') { 'STATIC' }
        elseif ($all -match 'ParseError') { 'PARSE' }
        elseif ($all -match '\b(TypeError|ValueError|AttributeError|NameError|IndexError|KeyError|TemplateError|ZeroDivisionError)\b') { 'RUNTIME' }
        else { 'NONE' }
    $msg = ''
    $m = [regex]::Match($all, '(?:StaticTypeError|ParseError)\s+(.+)')
    if ($m.Success) {
        $msg = $m.Groups[1].Value.Trim()
    } else {
        foreach ($line in ($all -split "`r?`n")) {
            $t = $line.Trim()
            if ($t -and $t -notmatch '^[-─\s]+$') { $msg = $t; break }
        }
    }
    return @{ Verdict = $verdict; Msg = $msg }
}

# 判定の強さ（退行判定用）。大きいほど強い。
$rank = @{ 'NONE' = 0; 'PARSE' = 0; 'TIMEOUT' = 0; 'RUNTIME' = 1; 'STATIC' = 2 }

$rows = @()
foreach ($p in $probes) {
    $text = [System.IO.File]::ReadAllText($p.FullName)
    $id = if ($text -match '(?m)^#\s*@id:\s*(\S+)') { $Matches[1] } else { $p.BaseName }
    $obl = if ($text -match '(?m)^#\s*@obligation:\s*(.+)$') { $Matches[1].Trim() } else { '(不明)' }
    $base = if ($text -match '(?m)^#\s*@baseline:\s*(\S+)') { $Matches[1] } else { 'NONE' }
    if ($Filter -and ($id -notlike "*$Filter*") -and ($obl -notlike "*$Filter*")) { continue }

    $r = Get-Verdict $p.FullName
    $rows += [pscustomobject]@{
        Id = $id; Obligation = $obl; Baseline = $base
        Verdict = $r.Verdict; Msg = $r.Msg; File = $p.FullName
    }
}

if ($rows.Count -eq 0) { Write-Host "no probes matched filter '$Filter'" -ForegroundColor Yellow; exit 0 }

$regressed = @($rows | Where-Object { $rank[$_.Verdict] -lt $rank[$_.Baseline] })
$progressed = @($rows | Where-Object { $rank[$_.Verdict] -gt $rank[$_.Baseline] })

$w = ($rows | ForEach-Object { $_.Obligation.Length } | Measure-Object -Maximum).Maximum
if ($ShowAll) {
    Write-Host ("{0,-5} {1,-$w} {2,-8} {3}" -f 'ID', '義務', '判定', '最初の診断')
    Write-Host ('-' * ($w + 70))
    foreach ($r in $rows) {
        $color = switch ($r.Verdict) { 'STATIC' { 'Green' } 'RUNTIME' { 'Yellow' } default { 'Red' } }
        $msg = if ($r.Msg.Length -gt 70) { $r.Msg.Substring(0, 70) } else { $r.Msg }
        Write-Host ("{0,-5} {1,-$w} {2,-8} {3}" -f $r.Id, $r.Obligation, $r.Verdict, $msg) -ForegroundColor $color
    }
    Write-Host ''
}

if ($regressed.Count -gt 0) {
    Write-Host 'REGRESSED (baseline より弱くなった — 検査が消えた):' -ForegroundColor Red
    foreach ($r in $regressed) {
        Write-Host ("  {0,-5} {1}  {2} -> {3}" -f $r.Id, $r.Obligation, $r.Baseline, $r.Verdict) -ForegroundColor Red
    }
    Write-Host ''
}
if ($progressed.Count -gt 0) {
    Write-Host 'PROGRESS (baseline より強くなった — -UpdateBaseline で焼き直す):' -ForegroundColor Cyan
    foreach ($r in $progressed) {
        Write-Host ("  {0,-5} {1}  {2} -> {3}" -f $r.Id, $r.Obligation, $r.Baseline, $r.Verdict) -ForegroundColor Cyan
    }
    Write-Host ''
}

if ($UpdateBaseline) {
    $n = 0
    foreach ($r in ($regressed + $progressed)) {
        $t = [System.IO.File]::ReadAllText($r.File)
        $t = [regex]::Replace($t, '(?m)^(#\s*@baseline:\s*)\S+', "`${1}$($r.Verdict)")
        # ⚠ UTF-8（BOM なし）で書く。PS 5.1 の Set-Content は ANSI 既定で日本語が壊れる
        [System.IO.File]::WriteAllText($r.File, $t, (New-Object System.Text.UTF8Encoding $false))
        $n++
    }
    Write-Host "updated @baseline on $n probe(s)" -ForegroundColor Cyan
}

$byVerdict = $rows | Group-Object Verdict | ForEach-Object { "$($_.Name)=$($_.Count)" }
$static = @($rows | Where-Object { $_.Verdict -eq 'STATIC' }).Count
Write-Host ("型義務 {0} 件: {1}" -f $rows.Count, ($byVerdict -join '  '))
Write-Host ("静的に検査されている割合: {0:P0}" -f ($static / $rows.Count))
if ($regressed.Count -gt 0) { Write-Host 'TYPE-OBLIGATIONS: REGRESSED' -ForegroundColor Red; exit 1 }
Write-Host 'TYPE-OBLIGATIONS: no regression' -ForegroundColor Green
exit 0
