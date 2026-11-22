# gate_judge.ps1 — 共通ランナー（gate_run.ps1）が保存した結果から合否を出す判定（gate_speed_plan.md 3-3）。dot-source して使う。
#
# 実行はしない（D-3）。scan_examples.ps1 / force_gate.ps1（と 5-3 のまとめた入口）が同じ判定を使う。
# ⚠ このファイルは日本語コメントを含むので UTF-8 BOM 付きで保存すること。

# 色の符号を除き、空白を詰める（今の scan_examples の FAIL 行と同じ形）。
function ConvertTo-GateOneLine([string]$Text) {
    return (($Text -replace ([string][char]27 + '\[[0-9;]*m'), '') -replace '\s+', ' ').Trim()
}

# scan: exit が 0 でないものを `FAIL<TAB>名前<TAB>stderr の末尾 3 行`、時間切れを `TIMEOUT<TAB>名前` で出す。
# 戻り値は出した行の数。$Results は表示の順に並べておく（今のスクリプトは名前の順）。
function Write-GateScanReport($Results) {
    $n = 0
    foreach ($r in $Results) {
        $name = Split-Path -Leaf $r.File
        if ($r.TimedOut) {
            Write-Host "TIMEOUT`t$name"
            $n++
            continue
        }
        if ($r.Exit -ne 0) {
            $err = Read-GateText $r.ErrPath
            $line = ($err -split "`n" | Where-Object { $_ -match '\S' } | Select-Object -Last 3) -join ' | '
            Write-Host "FAIL`t$name`t$(ConvertTo-GateOneLine $line)"
            $n++
        }
    }
    return $n
}

# force: stderr に `VmForceError: …` があるもの（1 ファイル最初の 1 件）。時間切れで止めたものも stderr を読む
# （止めた時点までに force の誤りが出ていないことは分かる・force_gate.ps1 の「サンプル判定」）。
function Get-GateForceHits($Results, [string]$RelBase) {
    $hits = @()
    foreach ($r in $Results) {
        foreach ($line in ((Read-GateText $r.ErrPath) -split "`n")) {
            if ($line -match 'VmForceError: (.+)$') {
                $rel = if ($RelBase -and $r.File.StartsWith($RelBase + '\')) { $r.File.Substring($RelBase.Length + 1) } else { $r.File }
                $hits += [pscustomobject]@{ File = $rel; What = ($Matches[1] -replace ([string][char]27 + '\[[0-9;]*m'), '').Trim() }
                break
            }
        }
    }
    return $hits
}

# gate/ の下の実行するテスト（lib/ と *_mods/ の下は除く・gate/README.md 1 節）。
function Get-GateTestFiles([string]$Root) {
    return @(Get-ChildItem -LiteralPath $Root -Filter *.ar -Recurse | Where-Object {
        $_.FullName -notmatch '\\lib\\' -and $_.FullName -notmatch '_mods\\'
    })
}

# ── 期待との照合（gate/README.md 3 節・gate_speed_plan.md D-14 / 3-4）──────────────────────────────

$script:GateStages = @('parse', 'expand', 'static', 'warning', 'runtime')

# テストのファイルから期待を読む。戻り値: @{ Items = @(@{Stage; Kind; Line; Fragment; Src}); Errors = @(読めなかった行) }
#   `# expect: <段階> <種類> ["断片"]`（`;` で複数）をその行の期待、`# expect-file: …` を位置の無い期待にする。
#   ⚠ 書式は厳しく読む（種類の後に置けるのは `"断片"` だけ）。読めない期待を黙って捨てると、禁止が通っても気づかない。
function Read-GateExpectations([string]$File) {
    $items = @(); $errors = @()
    $lines = [IO.File]::ReadAllLines($File, [Text.Encoding]::UTF8)
    for ($n = 0; $n -lt $lines.Count; $n++) {
        $m = [regex]::Match($lines[$n], '#\s*expect(-file)?:\s*(.*)$')
        if (-not $m.Success) { continue }
        $line = if ($m.Groups[1].Success -and $m.Groups[1].Value) { $null } else { $n + 1 }
        foreach ($part in ($m.Groups[2].Value -split ';')) {
            $q = [regex]::Match($part.Trim(), '^(\w+)\s+(-|[A-Za-z_][\w.]*)(?:\s+"([^"]*)")?$')
            if ((-not $q.Success) -or ($script:GateStages -notcontains $q.Groups[1].Value)) {
                $errors += ("line {0}: cannot read the expectation '{1}' (write: <stage> <kind> [""fragment""]; stage is one of {2})" -f ($n + 1), $part.Trim(), ($script:GateStages -join ' / '))
                continue
            }
            $kind = if ($q.Groups[2].Value -eq '-') { $null } else { $q.Groups[2].Value }
            $frag = if ($q.Groups[3].Success) { $q.Groups[3].Value } else { $null }
            $items += [pscustomobject]@{ Stage = $q.Groups[1].Value; Kind = $kind; Line = $line; Fragment = $frag; Src = $n + 1 }
        }
    }
    return [pscustomobject]@{ Items = $items; Errors = $errors }
}

# 診断の行を「テストのファイルの中の行」に直す。別のファイル（import 先）なら $null（expect-file とだけ照らす）。
# 実行時の誤りは、テストのファイル自身のいちばん内側のフレームの行（README 3.1）。
function Get-GateDiagLine($Diag, [string]$TestFile) {
    $dir = Split-Path -Parent $TestFile
    $same = {
        param($f)
        if (-not $f) { return $false }
        $full = if ([IO.Path]::IsPathRooted($f)) { $f } else { Join-Path $dir $f }
        try { return ([IO.Path]::GetFullPath($full) -eq [IO.Path]::GetFullPath($TestFile)) } catch { return $false }
    }
    if ($Diag.stage -eq 'runtime') {
        $frames = @($Diag.frames)
        for ($k = $frames.Count - 1; $k -ge 0; $k--) {
            $fr = $frames[$k]
            if (($null -ne $fr.line) -and (& $same $fr.file)) { return [int]$fr.line }
        }
        if (($null -ne $Diag.line) -and (& $same $Diag.file)) { return [int]$Diag.line }
        return $null
    }
    if (($null -ne $Diag.line) -and (& $same $Diag.file)) { return [int]$Diag.line }
    return $null
}

# 1 つのテストの結果を照らす。戻り値: @{ Pass; Status; Reasons = @() }
#   Status: PASS / CHECK-FAILED（確かめ落ち）/ FAILED（異常終了）/ EXPECT-MISMATCH / TIMEOUT / NO-DIAG（診断のファイルが無い）
function Test-GateResult($Result) {
    $reasons = @()
    if ($Result.TimedOut) { return [pscustomobject]@{ Pass = $false; Status = 'TIMEOUT'; Reasons = @('timed out') } }
    $exp = Read-GateExpectations $Result.File
    $diags = Read-GateDiag $Result.DiagPath
    if ($null -eq $diags) {
        return [pscustomobject]@{ Pass = $false; Status = 'NO-DIAG'; Reasons = @("exit $($Result.Exit) and no diagnostic file (arrow.exe stopped before writing it)") }
    }
    $err = Read-GateText $Result.ErrPath
    if ($err -match 'Warning: compiled module .* is out of date') { $reasons += 'a compiled module (.arc) was out of date and the source ran instead (README 5)' }
    foreach ($e in $exp.Errors) { $reasons += $e }
    $isErrorName = ([IO.Path]::GetFileNameWithoutExtension($Result.File) -match '_error$')
    $hasExp = ($exp.Items.Count -gt 0)
    if ($isErrorName -and -not $hasExp) { $reasons += 'the name ends with _error but there is no expectation (README 1)' }
    if ($hasExp -and -not $isErrorName) { $reasons += 'there are expectations but the name does not end with _error (README 1)' }

    if (-not $hasExp) {
        if ($Result.Exit -ne 0 -or $diags.Count -gt 0) {
            $status = 'FAILED'
            if (@($diags | Where-Object { $_.stage -eq 'runtime' -and $_.kind -eq 'GateCheckFailed' }).Count -gt 0) { $status = 'CHECK-FAILED' }
            foreach ($d in $diags) { $reasons += ("{0} {1} at line {2}: {3}" -f $d.stage, $(if ($d.kind) { $d.kind } else { '-' }), (Get-GateDiagLine $d $Result.File), $d.message) }
            if ($diags.Count -eq 0) { $reasons += "exit $($Result.Exit) without a diagnostic" }
            return [pscustomobject]@{ Pass = $false; Status = $status; Reasons = $reasons }
        }
        return [pscustomobject]@{ Pass = ($reasons.Count -eq 0); Status = $(if ($reasons.Count -eq 0) { 'PASS' } else { 'FAILED' }); Reasons = $reasons }
    }

    if ($Result.Exit -eq 0) { $reasons += 'exit 0: the forbidden code was accepted (expected it to stop)' }
    # 期待と診断の対応付け（断片のある期待から先に取る）
    $left = New-Object System.Collections.Generic.List[object]
    foreach ($d in $diags) { $left.Add([pscustomobject]@{ D = $d; Line = (Get-GateDiagLine $d $Result.File) }) }
    $ordered = @($exp.Items | Sort-Object @{ Expression = { if ($null -ne $_.Fragment) { 0 } else { 1 } } }, Src)
    foreach ($x in $ordered) {
        $hit = $null
        foreach ($c in $left) {
            if ($c.D.stage -ne $x.Stage) { continue }
            if ("$($c.D.kind)" -ne "$($x.Kind)") { continue }
            if ($c.Line -ne $x.Line) { continue }
            if (($null -ne $x.Fragment) -and (-not "$($c.D.message)".Contains($x.Fragment))) { continue }
            $hit = $c; break
        }
        if ($hit) { $null = $left.Remove($hit) }
        else {
            $where = if ($null -eq $x.Line) { 'the file' } else { "line $($x.Line)" }
            $reasons += ("missing: {0} {1}{2} at {3} (the forbidden code passed, or stopped for another reason)" -f $x.Stage, $(if ($x.Kind) { $x.Kind } else { '-' }), $(if ($null -ne $x.Fragment) { " ""$($x.Fragment)""" } else { '' }), $where)
        }
    }
    foreach ($c in $left) {
        $where = if ($null -eq $c.Line) { 'the file' } else { "line $($c.Line)" }
        $reasons += ("unexpected: {0} {1} at {2}: {3}" -f $c.D.stage, $(if ($c.D.kind) { $c.D.kind } else { '-' }), $where, $c.D.message)
    }
    return [pscustomobject]@{ Pass = ($reasons.Count -eq 0); Status = $(if ($reasons.Count -eq 0) { 'PASS' } else { 'EXPECT-MISMATCH' }); Reasons = $reasons }
}