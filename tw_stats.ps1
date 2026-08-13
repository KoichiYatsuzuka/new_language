# tw_stats.ps1 — 診断フック AR_TW_STATS=1 を全例題で回し、ツリーウォークの実行内訳を集計する（#10）。
#
# 「--vm=auto の実行中に、実際にツリーウォークへ落ちている文は何か」を測る。
# #3（強制バイトコード）へ残る距離は top-level のソース上の見た目ではなく、この実測でしか判らない。
#
# ⚠ フックは **feature `tw_stats` を付けたビッルドでしか存在しない**（既定ビルドでは
#    `enabled()` が定数 false になりコードごと消える）。env 判定だけにすると `exec()` の
#    1 文ごとに atomic 読みが残り 11% 退行するため、意図的にこの構成にしてある。
#    このスクリプトが専用ビルドを行う。
#
# 使い方: ./tw_stats.ps1 [-Timeout 20] [-SkipBuild]
param([int]$Timeout = 20, [switch]$SkipBuild)

$ErrorActionPreference = 'Stop'
if (-not $SkipBuild) {
    Write-Host "building with --features tw_stats ..." -ForegroundColor DarkGray
    # ⚠ `2>&1` を付けないこと。PS5.1 は native exe の stderr を ErrorRecord 化し、
    #    exit 0 でも失敗扱いになる（計画書の既知の落とし穴）。
    # cargo は進捗を stderr に出す。`$ErrorActionPreference=Stop` のままだと
    # PS5.1 がそれを終了エラー扱いにするので、この呼び出しの間だけ緩める。
    $prevEap = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    cargo build --features tw_stats | Out-Null
    $ErrorActionPreference = $prevEap
    if ($LASTEXITCODE -ne 0) { throw "build failed" }
}
$exe = Join-Path $PSScriptRoot 'target\debug\arrow.exe'
if (-not (Test-Path $exe)) { throw "not built: $exe" }

$files = Get-ChildItem -Path (Join-Path $PSScriptRoot 'examples') -Filter *.ar -Recurse |
         Where-Object { $_.FullName -notmatch '\\archived\\' }

$toplevel = @{}; $infn = @{}; $vmc = @{}; $bail = @{}
$ran = 0; $failed = 0

foreach ($f in $files) {
    $psi = New-Object System.Diagnostics.ProcessStartInfo
    $psi.FileName = $exe
    $psi.Arguments = "-src `"$($f.FullName)`" --vm=auto"
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
    if (-not $p.WaitForExit($Timeout * 1000)) { try { $p.Kill() } catch {}; $failed++; continue }
    $stderr = $errTask.Result
    [void]$outTask.Result
    $ran++

    foreach ($line in ($stderr -split "`n")) {
        if ($line -match '^TwStats\[(\w+)\] total=\d+ (.*)$') {
            $cat = $Matches[1]
            $tbl = switch ($cat) {
                'toplevel'   { $toplevel }
                'in_fn'      { $infn }
                'vm_compile' { $vmc }
                'vm_bail'    { $bail }
                default      { $null }
            }
            if ($null -eq $tbl) { continue }
            foreach ($pair in ($Matches[2].Trim() -split '\s+')) {
                if ($pair -match '^(.+)=(\d+)$') {
                    $k = $Matches[1]; $v = [long]$Matches[2]
                    if ($tbl.ContainsKey($k)) { $tbl[$k] += $v } else { $tbl[$k] = $v }
                }
            }
        }
    }
}

function Show-Table($name, $tbl) {
    $total = 0; foreach ($v in $tbl.Values) { $total += $v }
    Write-Host ""
    Write-Host "=== $name (total=$total) ===" -ForegroundColor Cyan
    if ($total -eq 0) { Write-Host "  (none)"; return }
    $tbl.GetEnumerator() | Sort-Object -Property Value -Descending | ForEach-Object {
        $pct = [math]::Round($_.Value * 100.0 / $total, 2)
        Write-Host ("{0,-26} {1,10}  {2,6}%" -f $_.Key, $_.Value, $pct)
    }
}

Write-Host "examples run: $ran (timeout/failed: $failed)"
Show-Table 'toplevel (module level tree-walk)' $toplevel
Show-Table 'in_fn (tree-walk function bodies)' $infn
Show-Table 'vm_compile (chunk compile outcome)' $vmc
Show-Table 'vm_bail (where the compiler gave up)' $bail
