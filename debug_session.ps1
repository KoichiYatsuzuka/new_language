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
# ⚠ stdin は BaseStream へ BOM 無しで書く（PS5.1 の StandardInput は BOM を付ける）。
function Invoke-Debug([string]$script, [string]$inputFile) {
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
        $wl = $want -split "`n"; $al = $actual -split "`n"
        for ($i = 0; $i -lt [Math]::Max($wl.Count, $al.Count); $i++) {
            $w = if ($i -lt $wl.Count) { $wl[$i] } else { '<missing>' }
            $a = if ($i -lt $al.Count) { $al[$i] } else { '<missing>' }
            if ($w -ne $a) { Write-Host ("    line {0}: want [{1}]  got [{2}]" -f ($i + 1), $w, $a) }
        }
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
