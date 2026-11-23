# gate_ab.ps1 — A/B 系（compare_outputs / compare_bytecode / compare_import_paths）の共通部分（gate_speed_plan.md 6-1〜6-3）。
# dot-source して使う（gate_run.ps1 / gate_judge.ps1 / cpp_cache_isolation.ps1 もここで読む）。
#
# ── 何をするか ──────────────────────────────────────────────────────────────
#   2 つの exe（A = 基準・B = 新しい）で `gate/` のテストを走らせ、その結果を組にして返す。比べ方（出力か・バイトコードか）は
#   呼ぶ側が決める。
#   - **実行は exe ごとに 1 回**。`Invoke-GateSuite` と同じ引数（VM のダンプ `AR_VM_DUMP=1` を付ける）で走らせるので、
#     compare_outputs と compare_bytecode は同じ結果を読み（1-3 でダンプが出力を変えないと確かめた）、B（target/release）は
#     「毎回」のゲート（gates_every_time）が残した結果を読むだけになる。
#   - **基準の exe の結果も exe のハッシュで保存する**（6-2）。2 回目からは新しい exe の実行だけになる。
#   - cpp-lib の中継 DLL のキャッシュ（D-11）: 走らせる exe だけを温め（`cpp_cache_isolation.ps1`）、終わったら始める前に戻す。
#     保存した結果が**冷えたキャッシュで取られていた**（`CppBridge:` の作り直しの行がある）ときは、その exe を温めて走らせ直す。
#     作り直しの表示が差に見えるため（7-34 で踏んだ）。
#   - **A と B が同じ exe（ハッシュが同じ）なら負の対照**: B を別の保存先（`target/gate_results_negctl`）に毎回走らせ直し、
#     独立した 2 回の実行を比べる。同じ保存を 2 回読んで「一致」と言わないため（6-4）。
#
# ⚠ このファイルは日本語コメントを含むので UTF-8 BOM 付きで保存すること。

. (Join-Path $PSScriptRoot 'gate_run.ps1')
. (Join-Path $PSScriptRoot 'gate_judge.ps1')
. (Join-Path $PSScriptRoot 'cpp_cache_isolation.ps1')

# 実行ごとに変わる値を潰す（compare_outputs の #63 の負の対照で判明。`<X object at 0x…>` の表示と `id()` の戻り値）。
# ⚠ 除外ではなく正規化にすること（除外すると、その出力を見る網が消える）。
function ConvertTo-GateAbStable([string]$Text) {
    $t = $Text -replace '0x[0-9a-fA-F]+', '0xADDR'
    return ($t -replace '(?<![\d.])\d{10,}(?![\d.])', '<ADDR>')
}

# 冷えたキャッシュで取られた結果（`CppBridge:` の行のうち `loading … (permanent)` でないものがある）の相対パス。
function Get-GateAbColdFiles($Results) {
    $cold = @()
    foreach ($r in $Results) {
        $err = Read-GateText $r.ErrPath
        if (-not $err.Contains('CppBridge:')) { continue }
        foreach ($line in ($err -split "`n")) {
            if ($line -match 'CppBridge:' -and $line -notmatch "CppBridge: loading '.*' \(permanent\)") { $cold += $r.Rel; break }
        }
    }
    return $cold
}

# 2 つの exe の結果を返す。戻り値: @{ A = 結果の配列; B = 結果の配列; NegativeControl = bool }
#   $Files を渡さなければ gate/ の全テスト（Invoke-GateSuite と同じ・保存を共有する）。渡せばその一覧（compare_import_paths）。
function Invoke-GateAbSides {
    param(
        [Parameter(Mandatory=$true)][string]$A,
        [Parameter(Mandatory=$true)][string]$B,
        [Parameter(Mandatory=$true)][string]$Root,
        [string[]]$Files = @(),
        [int]$Jobs = 0,
        [switch]$Fresh
    )
    $repo = Split-Path -Parent $PSScriptRoot
    $runArgs = Get-GateSuiteArgs $Root
    if ($Files.Count -gt 0) { $runArgs.Files = @($Files | ForEach-Object { (Resolve-Path -LiteralPath $_).Path }) }
    $negctl = ((Get-GateExeHash $A) -eq (Get-GateExeHash $B))
    $sides = @(
        [pscustomobject]@{ Name = 'A'; Exe = $A; RR = ''; Fresh = [bool]$Fresh; Res = $null }
        [pscustomobject]@{ Name = 'B'; Exe = $B; RR = $(if ($negctl) { Join-Path $repo 'target\gate_results_negctl' } else { '' }); Fresh = ([bool]$Fresh -or $negctl); Res = $null }
    )
    if ($negctl) {
        Write-Host 'gate_ab: A and B are the same exe — negative control: B runs again into target/gate_results_negctl (two independent runs)' -ForegroundColor DarkGray
    }
    # 保存を読めるか（冷えたキャッシュで取った結果は読まない）
    foreach ($s in $sides) {
        if ($s.Fresh) { continue }
        if (-not (Test-GateRunSaved -Exe $s.Exe @runArgs -ResultsRoot $s.RR)) { continue }
        $res = @(Invoke-GateRun -Exe $s.Exe @runArgs -Jobs $Jobs -ResultsRoot $s.RR)
        $cold = @(Get-GateAbColdFiles $res)
        if ($cold.Count -gt 0) {
            Write-Host ("gate_ab: the saved results of {0} were taken with a cold cpp-lib cache ({1}); running {0} again with a warm cache" -f $s.Name, ($cold -join ', ')) -ForegroundColor Yellow
            $s.Fresh = $true
        } else { $s.Res = $res }
    }
    $need = @($sides | Where-Object { $null -eq $_.Res })
    if ($need.Count -gt 0) {
        $tmp = Join-Path ([IO.Path]::GetTempPath()) ('ar_ab_' + [guid]::NewGuid().ToString('N'))
        $null = New-Item -ItemType Directory -Path $tmp
        $warm = @($runArgs.Files | Where-Object { [IO.File]::ReadAllText($_) -match 'import\[cpp-lib\]' })
        $cache = Initialize-CppCacheIsolation -Repo $runArgs.Root -Under @('.') -WorkDir file -Exes @($need | ForEach-Object { $_.Exe }) `
            -Examples $warm -Tmp $tmp -TimeoutSec $runArgs.TimeoutSec
        try {
            foreach ($s in $need) {
                Use-CppCache $cache $s.Exe
                $s.Res = @(Invoke-GateRun -Exe $s.Exe @runArgs -Jobs $Jobs -Fresh -ResultsRoot $s.RR)
                $cold = @(Get-GateAbColdFiles $s.Res)
                # ⚠ 温めた後も作り直すなら、比べても作り直しの表示が差に出る。黙って比べず知らせる。
                if ($cold.Count -gt 0) { Write-Host ("WARNING: {0} rebuilt a cpp-lib cache even after the warm-up ({1})" -f $s.Name, ($cold -join ', ')) -ForegroundColor Yellow }
            }
        } finally {
            Test-CppCacheUnchanged $cache
            Close-CppCacheIsolation $cache
            Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
        }
    }
    return [pscustomobject]@{ A = $sides[0].Res; B = $sides[1].Res; NegativeControl = $negctl }
}

# 出力の比較に使う 1 件の文字列（exit・stdout・ダンプを除いた stderr・機械向けの診断）。時間切れなら $null。
function Get-GateAbOutputRecord($R) {
    if ($R.TimedOut) { return $null }
    $out = (Read-GateText $R.OutPath) -replace "`r`n", "`n"
    $err = (Split-GateVmDump (Read-GateText $R.ErrPath)).Rest
    $diag = if (Test-Path -LiteralPath $R.DiagPath) { (Read-GateText $R.DiagPath) -replace "`r`n", "`n" } else { '<no diagnostic file>' }
    return (ConvertTo-GateAbStable ("EXIT=$($R.Exit)`n--- STDOUT ---`n$out`n--- STDERR ---`n$err`n--- DIAG ---`n$diag"))
}

# バイトコードの比較に使う 1 件の文字列（ダンプの塊を**並べ替えて**つないだもの）。時間切れなら $null。
#   ⚠ 塊の並びは比べない。async のタスクは worker スレッドがそれぞれコンパイルするので、同じ exe でも塊の出る順が変わる
#     （6-4 の負の対照で gate/ の async を使う 6 件が、行の集まりは同じで並びだけ違った。examples の 1-1 で
#     `interop/async_cs_calls` が 713 / 714 になったのも同じ）。並びはコンパイルの順＝実行の順で、実行の順が変われば
#     出力が変わるので compare_outputs が見る。塊の中身（命令の列）と塊の数は比べる。
function Get-GateAbDumpRecord($R) {
    if ($R.TimedOut) { return $null }
    $chunks = New-Object System.Collections.Generic.List[string]
    foreach ($c in ((Split-GateVmDump (Read-GateText $R.ErrPath)).Dump -split '(?m)(?=^== chunk )')) {
        if ($c -ne '') { $chunks.Add($c) }
    }
    $chunks.Sort([StringComparer]::Ordinal)
    return ($chunks -join '')
}

# 組にした結果を $Record（上の 2 つのどちらか）で比べて表示する。戻り値は exit code（差か時間切れがあれば 1）。
#   $Skip: 比べないファイルの相対パス → 理由（表示する。黙って外さない）。
function Compare-GateAbResults($Sides, [scriptblock]$Record, [string]$Label, [hashtable]$Skip = @{}, [switch]$ShowDiff, [int]$DiffLines = 20) {
    $same = 0; $diff = 0; $timeouts = 0; $skipped = 0
    $rows = @()
    for ($k = 0; $k -lt $Sides.A.Count; $k++) {
        $ra = $Sides.A[$k]; $rb = $Sides.B[$k]
        if ($ra.Rel -ne $rb.Rel) { throw "gate_ab: the two runs do not list the same files ($($ra.Rel) / $($rb.Rel))" }
        if ($Skip.ContainsKey($ra.Rel)) { $skipped++; $rows += ("{0,-60} SKIPPED ({1})" -f $ra.Rel, $Skip[$ra.Rel]); continue }
        $ta = & $Record $ra
        $tb = & $Record $rb
        if (($null -eq $ta) -or ($null -eq $tb)) { $timeouts++; $rows += ("{0,-60} TIMEOUT (not compared)" -f $ra.Rel); continue }
        if ($ta -ceq $tb) { $same++; continue }
        $diff++
        $rows += ("{0,-60} DIFFERS (A={1} lines, B={2} lines)" -f $ra.Rel, ($ta -split "`n").Count, ($tb -split "`n").Count)
        if ($ShowDiff) {
            $rows += (Compare-Object ($ta -split "`n") ($tb -split "`n") -CaseSensitive | Select-Object -First $DiffLines |
                ForEach-Object { "    {0} {1}" -f $_.SideIndicator, $_.InputObject })
        }
    }
    if ($rows.Count -gt 0) {
        Write-Host 'DIFFERING / NOT COMPARED:' -ForegroundColor Yellow
        $rows | ForEach-Object { Write-Host $_ }
        Write-Host ''
    }
    $what = if ($Sides.NegativeControl) { ' (negative control: same exe, two runs)' } else { '' }
    Write-Host ("{0} identical: {1} / {2}   differing: {3}   timeout: {4}   skipped: {5}{6}" -f $Label, $same, ($same + $diff), $diff, $timeouts, $skipped, $what)
    if (($diff -gt 0) -or ($timeouts -gt 0)) { return 1 }
    return 0
}

# 引数の exe を絶対パスにして確かめる（相対パスは Process.Start がカレントで解決して取り違える・タスク 9.5）。
function Resolve-GateAbExe([string]$Exe) {
    if (-not (Test-Path -LiteralPath $Exe)) { Write-Host "NOT FOUND: $Exe" -ForegroundColor Red; exit 2 }
    return (Resolve-Path -LiteralPath $Exe).Path
}
