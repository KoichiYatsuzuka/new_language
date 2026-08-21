# compare_bytecode.ps1 — 2 つの arrow.exe が生成する**バイトコードが同一か**を全例題で突き合わせる。
#
# 「挙動不変」の主張を exit code より強く裏付けるための検査（#52 で確立した手順の .ps1 化・#68）。
# リファクタリング（#62 / #63 / #66 …）のたびに手で回していたので固定した。
#
# 使い方:
#   ./compare_bytecode.ps1 -A <head-arrow.exe> -B <new-arrow.exe>
#   ./compare_bytecode.ps1 -A ../head_wt/target/release/arrow.exe   # -B の既定は target/release
#   ./compare_bytecode.ps1 -A x.exe -B x.exe                        # 負の対照（必ず 100% 一致）
#
# ⚠⚠ **async 例題は対象外**（#52）。worker スレッドの書き込み順とタスクのコンパイル回数が
#    スケジューリング依存なので、**同一バイナリでも dump が揺れる**。
#    差分を見たら**まず同一バイナリで再現するかを見る**（-A と -B に同じ exe を渡す）。
#
# ⚠ 子の stdout / stderr は **必ず ReadToEndAsync で同時に読む**（vm-pitfalls §4）。
#    AR_VM_DUMP=1 の出力は 1 例題で数 KB〜数十 KB 出るので、逐次 ReadToEnd にすると
#    パイプ（既定 4KB）が埋まって**確実にデッドロックする**。このスクリプト自身が
#    「AR_VM_DUMP を付けて回して確かめる」の負の対照になっている。
#
# ⚠ このファイルは**日本語コメントを含むので UTF-8 BOM 付きで保存すること**
#    （PS5.1 は BOM 無しを ANSI として読む）。
# ⚠ パス区切りは**全部スラッシュ**にしてある（生成スクリプト側でバックスラッシュが
#    エスケープに化けて壊した実績があるため。PowerShell はスラッシュを受け付ける）。
param(
    [Parameter(Mandatory=$true)][string]$A,
    [string]$B = '',
    [int]$TimeoutSec = 45,
    [switch]$ShowDiff
)

$ErrorActionPreference = 'Continue'
$repo = $PSScriptRoot
if ([string]::IsNullOrEmpty($B)) { $B = Join-Path $repo 'target/release/arrow.exe' }

foreach ($exe in @($A, $B)) {
    if (-not (Test-Path $exe)) { Write-Host "NOT FOUND: $exe" -ForegroundColor Red; exit 2 }
}

# GUI・対話・外部プロセス・長時間ベンチは対象外（dump を取る前に終わらない / 窓が出る）。
$skip = @(
    'debug_demo', 'async_bench', 'async_demo', 'spider_render', 'spider_solitaire',
    'rs_struct', 'flat_bench', 'flat_bench_interp', 'flat_bench_module',
    'cs_form_app', 'cs_proc_app', 'js_proc_test', 'js_proc_async_test', 'math_render',
    'importation', 'event_handler'
)
# ⚠ async ディレクトリは丸ごと対象外（同一バイナリでも揺れる・#52）。
$categoryDirs = @('basics','collections','classes','typing','exceptions','bench','apps','interop')

$examples = $categoryDirs | ForEach-Object {
    Get-ChildItem (Join-Path $repo "examples/$_/*.ar") -ErrorAction SilentlyContinue
} | Where-Object { $skip -notcontains $_.BaseName } | Sort-Object FullName

function Get-Dump([string]$exe, [string]$file) {
    $psi = New-Object System.Diagnostics.ProcessStartInfo
    $psi.FileName = $exe
    $psi.Arguments = '"' + $file + '"'
    $psi.WorkingDirectory = $repo
    $psi.UseShellExecute = $false
    $psi.RedirectStandardOutput = $true
    $psi.RedirectStandardError = $true
    $psi.CreateNoWindow = $true
    $psi.EnvironmentVariables['AR_VM_DUMP'] = '1'

    $proc = [System.Diagnostics.Process]::Start($psi)
    # ⚠ 逐次 ReadToEnd は子とデッドロックする（vm-pitfalls §4）。必ず両方を非同期で開始する。
    $outTask = $proc.StandardOutput.ReadToEndAsync()
    $errTask = $proc.StandardError.ReadToEndAsync()
    if (-not $proc.WaitForExit($TimeoutSec * 1000)) {
        try { $proc.Kill() } catch {}
        return $null
    }
    $null = $outTask.Result
    # バイトコードは stderr へ出る（AR_VM_DUMP=1）。行末差は正規化して比較する。
    return ($errTask.Result -replace "`r`n", "`n")
}

$same = 0; $diff = 0; $timeouts = 0
$rows = @()

foreach ($f in $examples) {
    $da = Get-Dump $A $f.FullName
    $db = Get-Dump $B $f.FullName
    if (($null -eq $da) -or ($null -eq $db)) {
        $timeouts++
        $rows += ("{0,-46} TIMEOUT (not compared)" -f $f.Name)
        continue
    }
    if ($da -eq $db) {
        $same++
    } else {
        $diff++
        $la = ($da -split "`n").Count
        $lb = ($db -split "`n").Count
        $rows += ("{0,-46} A={1} lines  B={2} lines" -f $f.Name, $la, $lb)
        if ($ShowDiff) {
            $rows += (Compare-Object ($da -split "`n") ($db -split "`n") |
                Select-Object -First 20 |
                ForEach-Object { "    {0} {1}" -f $_.SideIndicator, $_.InputObject })
        }
    }
}

if ($rows.Count -gt 0) {
    Write-Host 'DIFFERING / NOT COMPARED:' -ForegroundColor Yellow
    $rows | ForEach-Object { Write-Host $_ }
    Write-Host ''
}
Write-Host ("bytecode identical: {0} / {1}   differing: {2}   timeout: {3}" -f $same, ($same + $diff), $diff, $timeouts)
if ($diff -gt 0) { exit 1 } else { exit 0 }
