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
function Get-GateTreeFingerprint([string]$Root, [string]$Exclude) {
    $rootFull = (Resolve-Path -LiteralPath $Root).Path.TrimEnd('\')
    $lines = New-Object System.Collections.Generic.List[string]
    foreach ($f in [IO.Directory]::EnumerateFiles($rootFull, '*', [IO.SearchOption]::AllDirectories)) {
        $rel = $f.Substring($rootFull.Length + 1)
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

function Start-GateProcess($Exe, $File, $WorkDir, $ExtraEnv, $Out, $Err, $Diag) {
    $psi = New-Object System.Diagnostics.ProcessStartInfo
    $psi.FileName = $env:ComSpec
    # ⚠ cmd /c の引用符: 全体を "" で包み、中の各パスを " で包む（cmd の /s ではない既定の規則）
    $psi.Arguments = '/d /c ""' + $Exe + '" "' + $File + '" >"' + $Out + '" 2>"' + $Err + '""'
    $psi.WorkingDirectory = $WorkDir
    $psi.UseShellExecute = $false
    $psi.CreateNoWindow = $true
    foreach ($k in $ExtraEnv.Keys) { $psi.EnvironmentVariables[$k] = [string]$ExtraEnv[$k] }
    $psi.EnvironmentVariables['AR_DIAG_OUT'] = $Diag
    return [Diagnostics.Process]::Start($psi)
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
        [int]$Jobs = 1,
        # タイムアウトした後、窓を閉じる要求（CloseMainWindow）を送って自分で終わるのを待つ秒数（今の force_gate の GUI の例題・
        # 0 なら待たずに止める）。待つ間に終われば closed=1（終了処理まで確かめた）、終わらなければ止めて timeout=1。
        [int]$GraceSec = 0,
        [string]$TreeExclude = '',
        [switch]$Fresh,
        [string]$ResultsRoot = ''
    )
    $Exe = (Resolve-Path -LiteralPath $Exe).Path
    $rootFull = (Resolve-Path -LiteralPath $Root).Path.TrimEnd('\')
    if (-not $ResultsRoot) { $ResultsRoot = Join-Path (Split-Path -Parent $PSScriptRoot) 'target\gate_results' }
    $files = @($Files | ForEach-Object { (Resolve-Path -LiteralPath $_).Path })
    $rels = @($files | ForEach-Object { if ($_.StartsWith($rootFull + '\')) { $_.Substring($rootFull.Length + 1) } else { $_ } })
    $envText = (($ExtraEnv.Keys | Sort-Object | ForEach-Object { "$_=$($ExtraEnv[$_])" }) -join ';')
    $key = Get-GateStringHash ((Get-GateTreeFingerprint $rootFull $TreeExclude) + "`n" + ($rels -join "`n") + "`n" + $envText + "`n" + $WorkDir + "`n" + $TimeoutSec + "`n" + $GraceSec)
    $dir = Join-Path (Join-Path $ResultsRoot (Get-GateExeHash $Exe)) $key
    $index = Join-Path $dir 'index.csv'

    if ((-not $Fresh) -and (Test-Path -LiteralPath $index)) {
        $rows = @(Import-Csv -LiteralPath $index)
        if ($rows.Count -eq $files.Count) {
            Write-Host ("gate_run: reused {0} saved result(s) (exe {1}, {2}) — pass -Fresh to run again" -f $rows.Count, (Split-Path -Leaf (Split-Path -Parent $dir)), $key) -ForegroundColor DarkGray
            return @(ConvertTo-GateResults $rows $dir $files)
        }
    }
    if (Test-Path -LiteralPath $dir) { Remove-Item -LiteralPath $dir -Recurse -Force }
    $null = New-Item -ItemType Directory -Path $dir -Force

    $rows = New-Object System.Collections.Generic.List[object]
    $running = New-Object System.Collections.Generic.List[object]
    $next = 0
    $sw = [Diagnostics.Stopwatch]::StartNew()
    try {
        while (($next -lt $files.Count) -or ($running.Count -gt 0)) {
            while (($running.Count -lt [Math]::Max(1, $Jobs)) -and ($next -lt $files.Count)) {
                $i = $next + 1
                $stem = Join-Path $dir ('{0:D4}' -f $i)
                $wd = if ($WorkDir -eq 'file') { Split-Path -Parent $files[$next] } else { $WorkDir }
                $p = Start-GateProcess $Exe $files[$next] $wd $ExtraEnv "$stem.out" "$stem.err" "$stem.diag"
                $running.Add([pscustomobject]@{ I = $i; Rel = $rels[$next]; P = $p; Sw = [Diagnostics.Stopwatch]::StartNew(); Grace = $false; NextClose = 0.0 })
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
