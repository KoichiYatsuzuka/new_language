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
# ⚠⚠ **`archived/` と `practical_examples/` は意図的に対象外**（タスク 9.3・利用者の決定）。
#    このゲートを含め `$categoryDirs` を使う 4 本
#    （scan_examples / compare_outputs / compare_bytecode / compare_python_impl）は
#    9 カテゴリだけを走査する。
#    ⚠ `force_gate.ps1` と `compare_wasm_frontend.ps1` は `examples/` を `-Recurse` で
#      舐めるので上記 2 つも**含む**が、どちらも「例題が成功するか」は見ていない
#      （バイトコード化の可否／2 実装の診断の一致）。⇒ **壊れていても素通りする。**
#    ⚠⚠ **新しい検査の影響を測るときは母集団をこの 9 カテゴリに限ること。**
#      タスク 7.5 の偽陽性計測で `archived/` の壊れた例題を「偽陽性」と数えかけた。
#    理由と現状は examples/archived/README.md ・ examples/practical_examples/README.md。
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
