# gate_run.ps1 — ゲートの共通ランナー（gate_speed_plan.md 3-2）。dot-source して使う。
#
# ファイルを 1 回ずつ実行して exit / stdout / stderr / 機械向けの診断（AR_DIAG_OUT・3-1）/ 経過時間 / タイムアウトの有無を
# 保存するだけで、**合否は知らない**（D-3）。判定は各ゲートが保存した結果から出す。
#
# ── 保存（D-4 / D-6）────────────────────────────────────────────────────────
#   target/gate_results/<exe の中身のハッシュ>/<実行の鍵>/
#     index.csv            — 1 行 1 ファイル（i / rel / exit / timeout / ms）
#     NNNN.out / .err / .diag
#   実行の鍵 = 対象のディレクトリの下の全ファイルの（相対パス・大きさ・更新時刻）＋ 実行するファイルの一覧 ＋ 環境変数 ＋
#   作業ディレクトリの決め方 ＋ 制限時間 のハッシュ。⚠ exe だけを鍵にすると、テストや import 先を直した後も古い結果を
#   読んでしまう。⚠ 更新時刻で見るのは速さのため（examples/ は 1000 ファイル以上ある）。中身が変われば更新時刻も変わる。
#   保存があればそれを読み、**読んだことを必ず表示する**。`-Fresh` で全部実行し直す。
#
# ── 子プロセス（D-5・vm-pitfalls §4）──────────────────────────────────────────
#   - 出力はパイプで受けず、cmd のリダイレクトでファイルへ落とす（js-proc / cs-proc の孫プロセスがパイプを握る）。
#   - ファイルは共有モードで読む（孫プロセスが子の終わった後もファイルを握っていることがある・1-3）。
#   - 全部に CreateNoWindow を付ける。タイムアウトしたら子プロセスの木ごと止める（taskkill /T /F）。
#
# 使い方:
#   . (Join-Path $PSScriptRoot 'gate_run.ps1')
#   $res = Invoke-GateRun -Exe $exe -Root $root -Files $files [-WorkDir file|<dir>] [-ExtraEnv @{K='v'}] [-TimeoutSec 60] [-GraceSec 0] [-Jobs 1] [-Fresh]
#   foreach ($r in $res) { $r.Rel; $r.Exit; $r.TimedOut; $r.Closed; Read-GateText $r.OutPath; Read-GateDiag $r.DiagPath }
# ⚠ このファイルは日本語コメントを含むので UTF-8 BOM 付きで保存すること（PS5.1 は BOM 無しを ANSI で読む）。

function Get-GateStringHash([string]$Text) {
    $sha = [Security.Cryptography.SHA256]::Create()
    try { return (($sha.ComputeHash([Text.Encoding]::UTF8.GetBytes($Text)) | ForEach-Object { $_.ToString('x2') }) -join '').Substring(0, 16) }
    finally { $sha.Dispose() }
}

# exe の中身のハッシュ（パス・更新時刻ではない。同じパスに作り直した exe を取り違えないため・D-4）。
function Get-GateExeHash([string]$Exe) {
    return (Get-FileHash -LiteralPath $Exe -Algorithm SHA256).Hash.Substring(0, 16).ToLower()
}

# 対象のディレクトリの下の全ファイルの指紋。$Exclude（正規表現・相対パスに対して）に合うものは除く。
# ⚠ テストが実行中に書く一時ファイル（`_tmp_*`・.gitignore と同じ規則）は常に除く。入れると実行するたびに指紋が
#   変わり、保存した結果が二度と使われない（4-1 で gate/lang のファイル I/O のテストが踏んだ）。
# ⚠ cpp-lib の中継 DLL のキャッシュ（`ar_{stem}.dll` / `.syms` / `_shim.*`）も除く（6-2）。ヘッダ（指紋に入る）と exe（鍵に入る）
#   から作られるもので、A/B の比較は exe ごとにキャッシュを入れ替えて戻す（cpp_cache_isolation.ps1）。入れると、入れ替えや
#   作り直しのたびに鍵が変わり、基準の exe の保存が使われなくなる。
function Get-GateTreeFingerprint([string]$Root, [string]$Exclude) {
    $rootFull = (Resolve-Path -LiteralPath $Root).Path.TrimEnd('\')
    $lines = New-Object System.Collections.Generic.List[string]
    foreach ($f in [IO.Directory]::EnumerateFiles($rootFull, '*', [IO.SearchOption]::AllDirectories)) {
        $rel = $f.Substring($rootFull.Length + 1)
        $leaf = [IO.Path]::GetFileName($rel)
        if ($leaf.StartsWith('_tmp_')) { continue }
        if ($leaf -match '^ar_.+(\.dll|\.syms|_shim\.exp|_shim\.lib)$') { continue }
        if ($Exclude -and ($rel -match $Exclude)) { continue }
        $fi = New-Object IO.FileInfo($f)
        $lines.Add(('{0}|{1}|{2}' -f $rel, $fi.Length, $fi.LastWriteTimeUtc.Ticks))
    }
    $lines.Sort([StringComparer]::Ordinal)
    return (Get-GateStringHash ($lines -join "`n"))
}

# 子が書いたファイルを共有モードで読む（無ければ空文字列）。
function Read-GateText([string]$Path) {
    if (-not (Test-Path -LiteralPath $Path)) { return '' }
    $fs = New-Object IO.FileStream($Path, [IO.FileMode]::Open, [IO.FileAccess]::Read, ([IO.FileShare]::ReadWrite -bor [IO.FileShare]::Delete))
    try { $sr = New-Object IO.StreamReader($fs, [Text.Encoding]::UTF8); return $sr.ReadToEnd() } finally { $fs.Dispose() }
}

# 機械向けの診断（1 行 1 件の JSON）を読む。ファイルが無ければ $null（arrow.exe が書く前に止まった・3-1）。
function Read-GateDiag([string]$Path) {
    if (-not (Test-Path -LiteralPath $Path)) { return $null }
    $text = Read-GateText $Path
    # ⚠ `,` を付けて配列のまま返す（空の配列をそのまま return すると PowerShell が $null に崩し、「ファイルが無い」と区別できない）
    return ,@($text -split "`n" | Where-Object { $_.Trim() -ne '' } | ForEach-Object { $_ | ConvertFrom-Json })
}

# stderr を VM のダンプ（`AR_VM_DUMP=1`）とそれ以外に分ける（6-1）。戻り値: @{ Dump; Rest }（どちらも LF の文字列）。
#   ダンプの 1 つの塊 = `== chunk <名前> (n_locals=N) ==` の行・命令の行（`   0  CONST …`）・終わりの空行 1 つ
#   （src/vm/disasm.rs の `disassemble` を `eprintln!` で 1 回に出す）。
#   ⚠ 塊は 1 回の `eprintln!` で出るので、別のスレッドの出力が塊の途中に入ることはない（stderr の lock）。
#   ⚠ 命令の行に生の改行は入らない（定数は Rust の Debug 表示なので `\n` はエスケープされる）。この前提が崩れると、
#     塊の残りが Rest に漏れて compare_outputs が差を出す（黙って緩むのではなく赤になる）。
function Split-GateVmDump([string]$Text) {
    $dump = New-Object System.Text.StringBuilder
    $rest = New-Object System.Text.StringBuilder
    $inDump = $false
    foreach ($line in (($Text -replace "`r`n", "`n") -split "`n")) {
        if ($line -match '^== chunk .+ \(n_locals=\d+\) ==$') { $inDump = $true; $null = $dump.Append($line).Append("`n"); continue }
        if ($inDump) {
            if ($line -match '^\s*\d+\s+[A-Z_]') { $null = $dump.Append($line).Append("`n"); continue }
            $inDump = $false
            if ($line -eq '') { continue }   # 塊の終わりの空行
        }
        $null = $rest.Append($line).Append("`n")
    }
    $r = $rest.ToString()
    if ($r.EndsWith("`n")) { $r = $r.Substring(0, $r.Length - 1) }   # split で足した最後の改行を戻す
    return [pscustomobject]@{ Dump = $dump.ToString(); Rest = $r }
}

function Start-GateProcess($Exe, $File, $WorkDir, $ExtraEnv, $Out, $Err, $Diag) {
    $psi = New-Object System.Diagnostics.ProcessStartInfo
    $psi.FileName = $env:ComSpec
    # ⚠ cmd /c の引用符: 全体を "" で包み、中の各パスを " で包む（cmd の /s ではない既定の規則）
    # ⚠ 標準入力は NUL（EOF）。CreateNoWindow の子は隠れたコンソールから読むので、input() やデバッガの例題がそのまま止まった
    #   （3-3 で examples/debugger/ の 6 件が時間切れになった。古いゲートは親の標準入力＝EOF を引き継いでいた）。
    $psi.Arguments = '/d /c ""' + $Exe + '" "' + $File + '" <NUL >"' + $Out + '" 2>"' + $Err + '""'
    $psi.WorkingDirectory = $WorkDir
    $psi.UseShellExecute = $false
    $psi.CreateNoWindow = $true
    foreach ($k in $ExtraEnv.Keys) { $psi.EnvironmentVariables[$k] = [string]$ExtraEnv[$k] }
    $psi.EnvironmentVariables['AR_DIAG_OUT'] = $Diag
    return [Diagnostics.Process]::Start($psi)
}

# 実行の鍵と保存先（Invoke-GateRun と同じ引数を取る）。戻り値: @{ Files; Rels; Key; Dir; Index }
function Get-GateRunPlan {
    param(
        [Parameter(Mandatory=$true)][string]$Exe, [Parameter(Mandatory=$true)][string]$Root, [Parameter(Mandatory=$true)][string[]]$Files,
        [string]$WorkDir = 'file', [hashtable]$ExtraEnv = @{}, [int]$TimeoutSec = 60, [int]$GraceSec = 0,
        [string]$TreeExclude = '', [string]$ResultsRoot = ''
    )
    $rootFull = (Resolve-Path -LiteralPath $Root).Path.TrimEnd('\')
    if (-not $ResultsRoot) { $ResultsRoot = Join-Path (Split-Path -Parent $PSScriptRoot) 'target\gate_results' }
    $files = @($Files | ForEach-Object { (Resolve-Path -LiteralPath $_).Path })
    $rels = @($files | ForEach-Object { if ($_.StartsWith($rootFull + '\')) { $_.Substring($rootFull.Length + 1) } else { $_ } })
    $envText = (($ExtraEnv.Keys | Sort-Object | ForEach-Object { "$_=$($ExtraEnv[$_])" }) -join ';')
    $key = Get-GateStringHash ((Get-GateTreeFingerprint $rootFull $TreeExclude) + "`n" + ($rels -join "`n") + "`n" + $envText + "`n" + $WorkDir + "`n" + $TimeoutSec + "`n" + $GraceSec)
    $dir = Join-Path (Join-Path $ResultsRoot (Get-GateExeHash ((Resolve-Path -LiteralPath $Exe).Path))) $key
    return [pscustomobject]@{ Files = $files; Rels = $rels; Key = $key; Dir = $dir; Index = (Join-Path $dir 'index.csv') }
}

# 同じ引数の実行の結果が保存されているか（実行はしない。A/B 系が、走らせる exe だけ中継 DLL のキャッシュを温めるのに使う・6-2）。
function Test-GateRunSaved {
    param(
        [Parameter(Mandatory=$true)][string]$Exe, [Parameter(Mandatory=$true)][string]$Root, [Parameter(Mandatory=$true)][string[]]$Files,
        [string]$WorkDir = 'file', [hashtable]$ExtraEnv = @{}, [int]$TimeoutSec = 60, [int]$GraceSec = 0,
        [string]$TreeExclude = '', [string]$ResultsRoot = ''
    )
    $plan = Get-GateRunPlan -Exe $Exe -Root $Root -Files $Files -WorkDir $WorkDir -ExtraEnv $ExtraEnv -TimeoutSec $TimeoutSec -GraceSec $GraceSec -TreeExclude $TreeExclude -ResultsRoot $ResultsRoot
    if (-not (Test-Path -LiteralPath $plan.Index)) { return $false }
    return (@(Import-Csv -LiteralPath $plan.Index).Count -eq $plan.Files.Count)
}

function Invoke-GateRun {
    param(
        [Parameter(Mandatory=$true)][string]$Exe,
        [Parameter(Mandatory=$true)][string]$Root,
        [Parameter(Mandatory=$true)][string[]]$Files,
        # 'file' = 実行するファイルのディレクトリ（D-10）。それ以外はそのディレクトリ（今の scan_examples はリポジトリ直下）。
        [string]$WorkDir = 'file',
        [hashtable]$ExtraEnv = @{},
        [int]$TimeoutSec = 60,
        # 0 = 論理コアの半分（3-5）。1 で逐次。
        [int]$Jobs = 0,
        # タイムアウトした後、窓を閉じる要求（CloseMainWindow）を送って自分で終わるのを待つ秒数（今の force_gate の GUI の例題・
        # 0 なら待たずに止める）。待つ間に終われば closed=1（終了処理まで確かめた）、終わらなければ止めて timeout=1。
        [int]$GraceSec = 0,
        [string]$TreeExclude = '',
        [switch]$Fresh,
        [string]$ResultsRoot = ''
    )
    $Exe = (Resolve-Path -LiteralPath $Exe).Path
    $plan = Get-GateRunPlan -Exe $Exe -Root $Root -Files $Files -WorkDir $WorkDir -ExtraEnv $ExtraEnv -TimeoutSec $TimeoutSec -GraceSec $GraceSec -TreeExclude $TreeExclude -ResultsRoot $ResultsRoot
    $files = $plan.Files; $rels = $plan.Rels; $key = $plan.Key; $dir = $plan.Dir; $index = $plan.Index

    if ((-not $Fresh) -and (Test-Path -LiteralPath $index)) {
        $rows = @(Import-Csv -LiteralPath $index)
        if ($rows.Count -eq $files.Count) {
            Write-Host ("gate_run: reused {0} saved result(s) (exe {1}, {2}) — pass -Fresh to run again" -f $rows.Count, (Split-Path -Leaf (Split-Path -Parent $dir)), $key) -ForegroundColor DarkGray
            return @(ConvertTo-GateResults $rows $dir $files)
        }
    }
    if (Test-Path -LiteralPath $dir) { Remove-Item -LiteralPath $dir -Recurse -Force }
    $null = New-Item -ItemType Directory -Path $dir -Force

    if ($Jobs -le 0) { $Jobs = [Math]::Max(1, [int]([Environment]::ProcessorCount / 2)) }
    # ⚠ `import[cpp-lib]` を書いたファイルは先に 1 本ずつ走らせる（同じ中継 DLL をヘッダの隣に同時に作り直すと取り合う・
    #   gate/README.md 6 節）。それで中継 DLL のキャッシュが温まるので、残りは並列でよい。
    $serial = @{}
    if ($Jobs -gt 1) {
        for ($k = 0; $k -lt $files.Count; $k++) {
            if ([IO.File]::ReadAllText($files[$k]) -match 'import\[cpp-lib\]') { $serial[$k] = $true }
        }
    }
    $order = @(@(0..($files.Count - 1) | Where-Object { $serial.ContainsKey($_) }) + @(0..($files.Count - 1) | Where-Object { -not $serial.ContainsKey($_) }))
    if ($files.Count -eq 0) { $order = @() }
    $rows = New-Object System.Collections.Generic.List[object]
    $running = New-Object System.Collections.Generic.List[object]
    $next = 0
    $sw = [Diagnostics.Stopwatch]::StartNew()
    try {
        while (($next -lt $files.Count) -or ($running.Count -gt 0)) {
            while ($next -lt $files.Count) {
                $k = $order[$next]
                # 逐次のファイルの間（と、まだ走っている間）は 1 本だけ
                $limit = if ($serial.ContainsKey($k) -or @($running | Where-Object { $_.Serial }).Count -gt 0) { 1 } else { [Math]::Max(1, $Jobs) }
                if ($running.Count -ge $limit) { break }
                $i = $k + 1
                $stem = Join-Path $dir ('{0:D4}' -f $i)
                $wd = if ($WorkDir -eq 'file') { Split-Path -Parent $files[$k] } else { $WorkDir }
                $p = Start-GateProcess $Exe $files[$k] $wd $ExtraEnv "$stem.out" "$stem.err" "$stem.diag"
                $running.Add([pscustomobject]@{ I = $i; Rel = $rels[$k]; P = $p; Sw = [Diagnostics.Stopwatch]::StartNew(); Grace = $false; NextClose = 0.0; Serial = $serial.ContainsKey($k) })
                $next++
            }
            [Threading.Thread]::Sleep(10)
            foreach ($r in $running.ToArray()) {
                $done = $r.P.HasExited
                $timedOut = $false
                if ((-not $done) -and ($r.Sw.Elapsed.TotalSeconds -gt $TimeoutSec)) {
                    if ($r.Sw.Elapsed.TotalSeconds -le ($TimeoutSec + $GraceSec)) {
                        # 猶予の間: 子（cmd の下の arrow.exe）に窓を閉じる要求を送り続ける（1 秒に 1 回）
                        if ($r.Sw.Elapsed.TotalSeconds -ge $r.NextClose) {
                            $r.NextClose = $r.Sw.Elapsed.TotalSeconds + 1
                            if (-not $r.Grace) { $r.Grace = $true }
                            foreach ($c in @(Get-CimInstance Win32_Process -Filter "ParentProcessId=$($r.P.Id)" -ErrorAction SilentlyContinue)) {
                                try { $null = ([Diagnostics.Process]::GetProcessById([int]$c.ProcessId)).CloseMainWindow() } catch {}
                            }
                        }
                        continue
                    }
                    & taskkill /T /F /PID $r.P.Id *> $null
                    $null = $r.P.WaitForExit(5000)
                    $timedOut = $true
                }
                if ($done -or $timedOut) {
                    $null = $running.Remove($r)
                    $exit = if ($timedOut) { '' } else { [string]$r.P.ExitCode }
                    $rows.Add([pscustomobject]@{ i = $r.I; rel = $r.Rel; exit = $exit; timeout = [int][bool]$timedOut
                        closed = [int]($done -and $r.Grace); ms = [int]$r.Sw.Elapsed.TotalMilliseconds })
                }
            }
        }
    } finally {
        foreach ($r in $running.ToArray()) { & taskkill /T /F /PID $r.P.Id *> $null }
    }
    $sorted = @($rows | Sort-Object i)
    # ⚠ 全部そろったときだけ index を書く（途中で止まった実行を次回「保存あり」と読まないように）
    if ($sorted.Count -eq $files.Count) { $sorted | Export-Csv -LiteralPath $index -NoTypeInformation -Encoding UTF8 }
    Write-Host ("gate_run: ran {0} file(s) in {1:N1}s (jobs={2}, exe {3}, {4})" -f $files.Count, $sw.Elapsed.TotalSeconds, $Jobs, (Split-Path -Leaf (Split-Path -Parent $dir)), $key) -ForegroundColor DarkGray
    return @(ConvertTo-GateResults $sorted $dir $files)
}

function ConvertTo-GateResults($Rows, [string]$Dir, [string[]]$Files) {
    foreach ($row in $Rows) {
        $i = [int]$row.i
        $stem = Join-Path $Dir ('{0:D4}' -f $i)
        [pscustomobject]@{
            File = $Files[$i - 1]; Rel = $row.rel
            Exit = $(if ($row.exit -eq '') { $null } else { [int]$row.exit })
            TimedOut = ([int]$row.timeout -ne 0); Closed = ([int]$row.closed -ne 0); Ms = [int]$row.ms
            OutPath = "$stem.out"; ErrPath = "$stem.err"; DiagPath = "$stem.diag"
        }
    }
}
