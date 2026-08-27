# scan_examples.ps1 — 全例題を実行し、失敗したものだけを理由付きで列挙する。
#
# _archive/run_examples.ps1（退避済み）との違い:
#   - タイムアウト付き（GUI/外部プロセス例題でハングしない）
#   - System.Diagnostics.Process を直接使う（Start-Process -PassThru の ExitCode は当てにならない）
#   - 失敗した例題の stderr 末尾を 1 行に畳んで表示する
#
# 使い方: .\scripts\scan_examples.ps1
param([string]$Exe = '', [int]$TimeoutSec = 45)

$ErrorActionPreference = 'Continue'
$repo = Split-Path -Parent $PSScriptRoot   # scripts/ の 1 つ上 = リポジトリ直下
if ([string]::IsNullOrEmpty($Exe)) { $Exe = Join-Path $repo 'target\release\arrow.exe' }

$skip = @(
    'debug_demo', 'async_bench', 'async_demo', 'spider_render', 'spider_solitaire',
    'rs_struct', 'flat_bench', 'flat_bench_interp', 'flat_bench_module',
    'cs_form_app', 'cs_proc_app', 'js_proc_test', 'js_proc_async_test', 'math_render',
    'importation'
)
$categoryDirs = @('basics','collections','classes','typing','exceptions','async','bench','apps','interop')

$examples = $categoryDirs | ForEach-Object { Get-ChildItem "$repo\examples\$_\*.ar" -ErrorAction SilentlyContinue } | Where-Object {
    $n = $_.BaseName
    -not ($n -match '_error' -or $n -match '__errors' -or $skip -contains $n)
} | Sort-Object Name

foreach ($f in $examples) {
    $psi = New-Object System.Diagnostics.ProcessStartInfo
    $psi.FileName = $Exe
    $psi.Arguments = '"' + $f.FullName + '"'
    $psi.WorkingDirectory = $repo
    $psi.UseShellExecute = $false
    $psi.RedirectStandardOutput = $true
    $psi.RedirectStandardError = $true
    $psi.CreateNoWindow = $true

    $proc = [System.Diagnostics.Process]::Start($psi)
    # 出力バッファ充填でのデッドロックを避けるため非同期で読む
    $outTask = $proc.StandardOutput.ReadToEndAsync()
    $errTask = $proc.StandardError.ReadToEndAsync()

    if (-not $proc.WaitForExit($TimeoutSec * 1000)) {
        try { $proc.Kill() } catch {}
        Write-Host "TIMEOUT`t$($f.Name)"
        continue
    }
    $code = $proc.ExitCode
    if ($code -ne 0) {
        $err = $errTask.Result
        $line = ($err -split "`n" | Where-Object { $_ -match '\S' } | Select-Object -Last 3) -join ' | '
        $line = ($line -replace '\x1b\[[0-9;]*m', '') -replace '\s+', ' '
        Write-Host "FAIL`t$($f.Name)`t$($line.Trim())"
    }
}
Write-Host "SCAN-DONE"
