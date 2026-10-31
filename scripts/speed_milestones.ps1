# speed_milestones.ps1 — 計画書のマイルストーン（初期・途中経過・現在）の exe を作り、同じセッションで速度を並べて比べる
# （native_object_access_plan.md §0.7）
#
#   ./scripts/speed_milestones.ps1                       # 既定のマイルストーンで区間（bench.ar）と 1 操作の単価（micro）を測る
#   ./scripts/speed_milestones.ps1 -Worktree <dir>       # 作業用の worktree（無ければ git worktree add で作る）
#   ./scripts/speed_milestones.ps1 -Milestones a=<sha>,b=HEAD -Modes native
#   ./scripts/speed_milestones.ps1 -SkipBuild            # 前に作った exe（arrow_<ラベル>.exe）をそのまま使う
#   ./scripts/speed_milestones.ps1 -TableOnly            # 測らずに、-OutDir の CSV から並べた表（milestones.md）だけ作る
#
# 仕組み:
#   * マイルストーンごとに worktree をその commit にして `cargo build --release` し、exe を同じディレクトリに
#     `arrow_<ラベル>.exe` として写す（exe の隣の資源を引く場合に備えて同じ場所に置く）。HEAD も worktree で作る
#     （作業ツリーの未コミットの変更は入らない）
#   * 測るのは scripts/speed_modes.ps1 の `-Exes`（全部の exe を交互に走らせ、中央値を横に並べる・最後の列は
#     最初 ÷ 最後）。例題は**今の** examples/bench/speed_modes を使う（各 exe が自分で .arc を作る）
#   * 結果は -OutDir に区間・単価の CSV（speed_modes の -Csv）とログを残す
#   * 1 回のビルドは -BuildLimitMin で打ち切り、プロセスの木ごと止めて ABNORMAL TERMINATION を出す
#
# ⚠ 1 つの commit の差を見るなら speed_modes.ps1 -B（2 つの exe）を使う。ここは数字を並べて推移を見る用。
# ⚠ ノイズの幅は同じセッションで `-Milestones x=<sha>,y=<同じ sha>` を測れば分かる（vm-pitfalls §1）。

param(
    [string[]]$Milestones = @(
        'p0_init=326e112',   # 計画の着手前
        'p1_ph7=b8cf9fb',    # フェーズ 1〜7 完了（§0.4）
        'p2_ph8=daa11bc',    # フェーズ 8 完了（8-2）
        'p3_ph10=b20f63b',   # フェーズ 9・10 完了（§0.6）
        'p4_now=HEAD'        # 今の HEAD
    ),
    [string]$Worktree = '',
    [string[]]$Modes = @('interp', 'native', 'cpp'),
    [int]$Reps = 7,
    [int]$Scale = 5,
    [switch]$SkipBuild,
    [switch]$SkipMicro,
    [switch]$TableOnly,
    [int]$BuildLimitMin = 60,
    [string]$OutDir = ''
)

$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
if ($Worktree -eq '') { $Worktree = Join-Path (Split-Path -Parent $repo) 'arrow_milestones' }
if ($OutDir -eq '') { $OutDir = Join-Path $env:TEMP 'arrow_speed_milestones' }
New-Item -ItemType Directory -Force $OutDir | Out-Null

# ── マイルストーンの表（ラベル → commit） ───────────────────────────────────────────
$ms = [ordered]@{}
foreach ($item in $Milestones) {
    $i = $item.IndexOf('=')
    if ($i -le 0) { throw "-Milestones takes label=commit: $item" }
    $label = $item.Substring(0, $i)
    if ($label -notmatch '^[A-Za-z0-9_-]+$') { throw "label must be [A-Za-z0-9_-]: $label" }
    $rev = & git -C $repo rev-parse --verify --quiet ($item.Substring($i + 1) + '^{commit}')
    if ($LASTEXITCODE -ne 0 -or -not $rev) { throw "unknown commit: $($item.Substring($i + 1))" }
    $ms[$label] = $rev.Trim()
}

# ── worktree ─────────────────────────────────────────────────────────────────────
if (-not (Test-Path (Join-Path $Worktree '.git'))) {
    Write-Host "creating worktree $Worktree"
    & git -C $repo worktree add --detach $Worktree $ms[@($ms.Keys)[0]] | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "git worktree add failed: $Worktree" }
}
$relDir = Join-Path $Worktree 'target\release'

function Build-At([string]$label, [string]$rev) {
    $dst = Join-Path $relDir "arrow_$label.exe"
    if ($SkipBuild -and (Test-Path $dst)) { return $dst }
    $prev = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    & git -C $Worktree checkout --quiet --detach $rev 2>&1 | Out-Null
    $ErrorActionPreference = $prev
    if ($LASTEXITCODE -ne 0) { throw "checkout failed: $rev" }
    $psi = New-Object System.Diagnostics.ProcessStartInfo
    $psi.FileName = 'cargo'; $psi.Arguments = 'build --release'
    $psi.WorkingDirectory = $Worktree
    $psi.UseShellExecute = $false; $psi.RedirectStandardOutput = $true; $psi.RedirectStandardError = $true
    $sw = [Diagnostics.Stopwatch]::StartNew()
    $p = [Diagnostics.Process]::Start($psi)
    $o = $p.StandardOutput.ReadToEndAsync(); $e = $p.StandardError.ReadToEndAsync()
    if (-not $p.WaitForExit($BuildLimitMin * 60 * 1000)) {
        & taskkill /T /F /PID $p.Id | Out-Null
        Write-Host "ABNORMAL TERMINATION: build of $label ($rev) exceeded $BuildLimitMin min (killed)" -ForegroundColor Red
        exit 1
    }
    $p.WaitForExit()
    if ($p.ExitCode -ne 0) {
        Write-Host "BUILD FAILED $label ($rev)`n$($e.Result)" -ForegroundColor Red
        exit 1
    }
    Copy-Item (Join-Path $relDir 'arrow.exe') $dst -Force
    Write-Host ("built {0,-10} {1}  ({2:N0}s)" -f $label, $rev.Substring(0, 9), $sw.Elapsed.TotalSeconds)
    return $dst
}

$exeArgs = @()
foreach ($label in $ms.Keys) {
    if ($TableOnly) { break }
    $exe = Build-At $label $ms[$label]
    $exeArgs += "$label=$exe"
}
Write-Host ''
foreach ($label in $ms.Keys) {
    $subject = & git -C $repo log -1 --format=%s $ms[$label]
    Write-Host ("{0,-10} {1}  {2}" -f $label, $ms[$label].Substring(0, 9), $subject)
}

# ── 測る（区間・1 操作の単価） ─────────────────────────────────────────────────────
$sm = Join-Path $PSScriptRoot 'speed_modes.ps1'
if (-not $TableOnly) {
& $sm -Exes $exeArgs -Modes $Modes -Reps $Reps -Scale $Scale -Csv (Join-Path $OutDir 'sections.csv')
if (-not $SkipMicro) {
    # 1 操作の単価は interp / native だけ（cpp の単価 micro_ffi_bench は FFI の境界の費用で、この比較の対象外）
    $microModes = @($Modes | Where-Object { $_ -ne 'cpp' })
    if ($microModes.Count -gt 0) {
        & $sm -Exes $exeArgs -Modes $microModes -Reps $Reps -Scale $Scale -Micro -Csv (Join-Path $OutDir 'micro.csv')
    }
}
}
# ── 並べた表（Markdown・計画書に貼れる形） ─────────────────────────────────────────
# 中央値をマイルストーンごとに横に並べ、最後の列は「最初 ÷ 最後」（1 より大きいほど速くなった）
$md = @("# speed milestones ($(Get-Date -Format 'yyyy-MM-dd'))", '')
foreach ($label in $ms.Keys) { $md += "- ``$label`` = ``$($ms[$label].Substring(0, 7))``" }
foreach ($f in 'sections', 'micro') {
    $csv = Join-Path $OutDir "$f.csv"
    if (-not (Test-Path $csv)) { continue }
    $data = Import-Csv $csv
    foreach ($mode in @($data | ForEach-Object { $_.mode } | Select-Object -Unique)) {
        $unit = 's'
        if ($f -eq 'micro') { $unit = 'ns' }
        $labels = @($ms.Keys)
        $md += ''
        $md += "## $f / $mode ($unit, median)"
        $md += ''
        $md += '| metric | ' + ($labels -join ' | ') + " | $($labels[0]) / $($labels[-1]) |"
        $md += '|---|' + (($labels | ForEach-Object { '---:' }) -join '|') + '|---:|'
        $rowsOf = $data | Where-Object { $_.mode -eq $mode }
        foreach ($metric in @($rowsOf | ForEach-Object { $_.metric } | Select-Object -Unique)) {
            $vals = foreach ($label in $labels) {
                $r = $rowsOf | Where-Object { $_.metric -eq $metric -and $_.exe -eq $label } | Select-Object -First 1
                if ($r) { [double]$r.median } else { $null }
            }
            $cells = foreach ($v in $vals) { if ($null -eq $v) { '-' } elseif ($unit -eq 'ns') { '{0:N1}' -f $v } else { '{0:N3}' -f $v } }
            $ratio = '-'
            if ($null -ne $vals[0] -and $null -ne $vals[-1] -and $vals[-1] -gt 0 -and $vals[0] -gt 0) { $ratio = '{0:N2}x' -f ($vals[0] / $vals[-1]) }
            $md += "| $metric | " + ($cells -join ' | ') + " | $ratio |"
        }
    }
}
$mdPath = Join-Path $OutDir 'milestones.md'
$md | Set-Content -Path $mdPath -Encoding utf8
Write-Host "table: $mdPath"
Write-Host "results: $OutDir"
Write-Host 'SPEED-MILESTONES-DONE'
