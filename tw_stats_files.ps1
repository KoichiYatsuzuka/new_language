# tw_stats_files.ps1 — #10 計測の補助: 例題ごとの内訳を出す（集中度・失敗例題の確認用）。
# ⚠ `tw_stats.ps1` と同じく feature `tw_stats` 付きビルドが要る（既定ビルドではフックが消える）。
param([int]$Timeout = 20, [switch]$SkipBuild)

$ErrorActionPreference = 'Stop'
if (-not $SkipBuild) {
    # ⚠ `2>&1` は付けない（PS5.1 が ErrorRecord 化して exit 0 でも失敗扱いになる）。
    # cargo は進捗を stderr に出す。`$ErrorActionPreference=Stop` のままだと
    # PS5.1 がそれを終了エラー扱いにするので、この呼び出しの間だけ緩める。
    $prevEap = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    cargo build --features tw_stats | Out-Null
    $ErrorActionPreference = $prevEap
    if ($LASTEXITCODE -ne 0) { throw "build failed" }
}
$exe = Join-Path $PSScriptRoot 'target\debug\arrow.exe'
$files = Get-ChildItem -Path (Join-Path $PSScriptRoot 'examples') -Filter *.ar -Recurse |
         Where-Object { $_.FullName -notmatch '\\archived\\' }

$rows = @()
foreach ($f in $files) {
    $psi = New-Object System.Diagnostics.ProcessStartInfo
    $psi.FileName = $exe
    $psi.Arguments = "-src `"$($f.FullName)`""
    $psi.WorkingDirectory = $f.DirectoryName
    $psi.RedirectStandardOutput = $true
    $psi.RedirectStandardError = $true
    $psi.UseShellExecute = $false
    $psi.EnvironmentVariables['AR_TW_STATS'] = '1'
    $p = New-Object System.Diagnostics.Process
    $p.StartInfo = $psi
    [void]$p.Start()
    $errTask = $p.StandardError.ReadToEndAsync()
    $outTask = $p.StandardOutput.ReadToEndAsync()
    $ok = $p.WaitForExit($Timeout * 1000)
    if (-not $ok) { try { $p.Kill() } catch {} }
    $stderr = if ($ok) { $errTask.Result } else { '' }
    if ($ok) { [void]$outTask.Result }
    $tl = 0; $fnf = 0
    foreach ($line in ($stderr -split "`n")) {
        if ($line -match '^TwStats\[toplevel\] total=(\d+)') { $tl = [long]$Matches[1] }
        if ($line -match 'fn_FAILED=(\d+)') { $fnf = [long]$Matches[1] }
    }
    $rows += [pscustomobject]@{
        File = $f.FullName.Substring($PSScriptRoot.Length + 1)
        Status = if ($ok) { "exit$($p.ExitCode)" } else { 'TIMEOUT' }
        Toplevel = $tl
        FnFailed = $fnf
    }
}

Write-Host "=== toplevel tree-walk stmts: top 15 ===" -ForegroundColor Cyan
$rows | Sort-Object Toplevel -Descending | Select-Object -First 15 | Format-Table -AutoSize
Write-Host "=== fn VM-compile failures: nonzero ===" -ForegroundColor Cyan
$rows | Where-Object { $_.FnFailed -gt 0 } | Sort-Object FnFailed -Descending | Format-Table -AutoSize
Write-Host "=== not exit0 ===" -ForegroundColor Cyan
$rows | Where-Object { $_.Status -ne 'exit0' } | Format-Table -AutoSize
