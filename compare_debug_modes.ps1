# compare_debug_modes.ps1 — 対話デバッガのステッピングが `--vm=off` と `--vm=auto` で
# byte-identical かを検証する（タスク #1）。
#
# なぜ別スクリプトなのか:
#   compare_vm_modes.ps1 は **stdin を与えない**ので、`break_point` を含む例題は比較できず
#   skip リストに入っている（`debug_demo`）。その結果、
#   「デバッグ中は VM を無効化してツリーウォークに委ねる」という #1 の暫定対応は
#   **一度も自動検証されていなかった**。ここがその穴を塞ぐ。
#
# 仕組み:
#   examples/debugger/<name>.ar と同名の <name>.in（デバッガへ流すコマンド列）を対にして、
#   両モードで実行し stdout+stderr を比較する。`.in` が無い .ar はスキップする。
#
# ⚠ このスクリプトは **#1（VM 内ステートメント単位ブレーク）に着手するときの安全網**でもある。
#   VM ディスパッチループへ停止判定を入れる変更は、ここが緑であることを条件に進めること。
#
# 使い方:
#   .\compare_debug_modes.ps1
#   .\compare_debug_modes.ps1 -Filter step_into
#   .\compare_debug_modes.ps1 -ShowDiff        # 不一致時に差分行も表示

param(
    [string]$Filter = '',
    [int]$TimeoutSec = 60,
    [switch]$ShowDiff
)

$ErrorActionPreference = 'Continue'

$repo = $PSScriptRoot
$exe = Join-Path $repo 'target\release\arrow.exe'
if (-not (Test-Path $exe)) { throw "not built: $exe (run: cargo build --release)" }

$dir = Join-Path $repo 'examples\debugger'
if (-not (Test-Path $dir)) { throw "not found: $dir" }

# 1 本走らせて stdout+stderr を返す（stdin に $inputFile の内容を流す）。
# Start-Process -PassThru の ExitCode は当てにならないので Process を直接使う。
function Invoke-Debug([string]$mode, [string]$script, [string]$inputFile) {
    $psi = New-Object System.Diagnostics.ProcessStartInfo
    $psi.FileName = $exe
    $psi.Arguments = "--vm=$mode -src `"$script`""
    $psi.WorkingDirectory = $repo
    $psi.UseShellExecute = $false
    $psi.RedirectStandardInput = $true
    $psi.RedirectStandardOutput = $true
    $psi.RedirectStandardError = $true
    $psi.CreateNoWindow = $true

    $proc = [System.Diagnostics.Process]::Start($psi)
    # 出力バッファ充填でのデッドロックを避けるため非同期で読む
    $outTask = $proc.StandardOutput.ReadToEndAsync()
    $errTask = $proc.StandardError.ReadToEndAsync()
    $proc.StandardInput.Write([System.IO.File]::ReadAllText($inputFile))
    $proc.StandardInput.Close()

    if (-not $proc.WaitForExit($TimeoutSec * 1000)) {
        try { $proc.Kill() } catch {}
        return "<<TIMEOUT>>"
    }
    $out = $outTask.Result
    $err = $errTask.Result
    # 一時ファイルの絶対パスは環境依存なので比較対象から外す（両モードで同じだが再現性のため）
    $text = "$out`n--stderr--`n$err"
    return $text -replace [regex]::Escape($repo), '<REPO>'
}

$scripts = Get-ChildItem "$dir\*.ar" | Where-Object {
    ($Filter -eq '') -or ($_.BaseName -like "*$Filter*")
} | Sort-Object Name

$identical = 0; $differing = 0; $skipped = 0
foreach ($s in $scripts) {
    $inFile = Join-Path $dir "$($s.BaseName).in"
    if (-not (Test-Path $inFile)) {
        Write-Host "SKIP (no .in)  $($s.Name)" -ForegroundColor DarkGray
        $skipped++
        continue
    }
    $off = Invoke-Debug 'off' $s.FullName $inFile
    $auto = Invoke-Debug 'auto' $s.FullName $inFile

    if ($off -eq $auto) {
        $identical++
    } else {
        $differing++
        Write-Host "DIFFER  $($s.Name)" -ForegroundColor Red
        if ($ShowDiff) {
            $d = Compare-Object ($off -split "`n") ($auto -split "`n")
            $d | Select-Object -First 20 | ForEach-Object {
                Write-Host ("  {0} {1}" -f $_.SideIndicator, $_.InputObject)
            }
        }
    }
}

Write-Host ""
Write-Host "identical: $identical   differing: $differing   skipped: $skipped"
