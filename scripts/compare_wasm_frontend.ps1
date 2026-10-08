# compare_wasm_frontend.ps1 -- verify the wasm editor frontend agrees with arrow.exe.
#
# The VS Code extension analyses .ar files with crates/arrow-frontend compiled to wasm.
# It runs the same lexer / parser / import code / type checker as arrow.exe; imports are read
# through the host (vscode-extension/src/wasm_host.ts), which this gate uses too (dump_diags.js).
# So the required property is: for every example, the two report the SAME static errors in
# that file (editor_import_resolution_plan.md 3-3). arrow.exe is run with AR_CHECK_ONLY=1, which
# stops after the static checks, so no example is executed (no GUI, FFI or sleeps).
#
#   * INVENTED (wasm only) and MISSED (arrow.exe only) must both be 0.
#   * Errors arrow.exe reports in an imported module's file are not compared: the editor shows a
#     document's own errors only.
#   * An import arrow.exe cannot resolve stops it with a ParseError; the editor reports the import
#     and goes on. Then only the import's message is compared (see the branch below).
#
# Until 3-3 the editor did not read imports, and the rule was "wasm may report fewer".
#
# ASCII-only on purpose: PowerShell 5.1 reads BOM-less .ps1 as ANSI.

param(
    [int]$TimeoutSec = 30,
    [switch]$VerboseDiff
)

$ErrorActionPreference = 'Continue'
$repo = Split-Path -Parent $PSScriptRoot
$exe  = Join-Path $repo 'target\release\arrow.exe'
$wasm = Join-Path $repo 'crates\arrow-frontend\target\wasm32-unknown-unknown\release\arrow_frontend.wasm'
$dump = Join-Path $repo 'crates\arrow-frontend\dump_diags.js'

if (-not (Test-Path $exe))  { throw "not built: $exe  (cargo build --release)" }
if (-not (Test-Path $wasm)) { throw "not built: $wasm (cd crates/arrow-frontend; cargo build --release --target wasm32-unknown-unknown)" }
if (-not (Test-Path $dump)) { throw "missing helper: $dump" }

# --- crates/arrow-frontend の単体テスト -------------------------------------
# ⚠⚠ **ここで走らせないと誰も走らせない。** `crates/arrow-frontend` は
#   `Cargo.toml` の `exclude` でワークスペース外なので、**root の `cargo test` にも
#   `test_build_gate.ps1` にも入らない**。実際 2026-09-19 のタスク 9.7 で
#   `type_refs::generic_arguments_of_unknown_base`（当時 `skipped_generic_arguments`）を
#   壊したまま、全ゲートが緑で通った。
# ⚠ このゲートは「lexer / parser / type_check を触ったとき」に走らせる約束なので、
#   同じ条件で守りたいテストの置き場としてちょうどよい。
# ⚠ 比較本体（wasm と exe の診断突き合わせ）とは**別の網**。こちらは
#   「型参照の索引」「スタブ解析」などの単体の性質を見る。
Write-Host 'running crates/arrow-frontend unit tests ...' -ForegroundColor Cyan
Push-Location (Join-Path $repo 'crates\arrow-frontend')
$feTest = & cargo test 2>&1
$feCode = $LASTEXITCODE
Pop-Location
if ($feCode -ne 0) {
    $feTest | ForEach-Object { Write-Host $_ }
    Write-Host 'FRONTEND-TESTS: FAILED' -ForegroundColor Red
    exit 1
}
# 0 件のスイートは数えても意味がないので、通ったテスト数の合計だけ出す。
$fePassed = ($feTest | Select-String -Pattern 'test result: ok\. (\d+) passed' -AllMatches |
    ForEach-Object { $_.Matches } | ForEach-Object { [int]$_.Groups[1].Value } |
    Measure-Object -Sum).Sum
Write-Host "frontend tests ok  ($fePassed passed)" -ForegroundColor Green

# --- the extension's host (dump_diags.js drives the wasm through it) ---------
# dump_diags.js loads vscode-extension/out/frontend.js + wasm_host.js, so the gate runs the wasm
# with the SAME host functions as the extension (the file reads behind imports). Compile it on
# every run: a stale out/ would test an old host.
Write-Host 'compiling vscode-extension (tsc) ...' -ForegroundColor Cyan
$tsc = & npm --prefix (Join-Path $repo 'vscode-extension') run compile 2>&1
if ($LASTEXITCODE -ne 0) {
    $tsc | ForEach-Object { Write-Host $_ }
    Write-Host 'EXTENSION-COMPILE: FAILED' -ForegroundColor Red
    exit 1
}

# --- locate a Node that understands modern wasm opcodes -----------------------
# The node on PATH may be ancient; VS Code ships a recent one and is always present
# on a machine that runs this extension.
function Get-NodeRunner {
    $codeCandidates = @(
        "$env:LOCALAPPDATA\Programs\Microsoft VS Code\Code.exe",
        "$env:ProgramFiles\Microsoft VS Code\Code.exe"
    )
    foreach ($c in $codeCandidates) {
        if (Test-Path $c) { return @{ Exe = $c; Electron = $true } }
    }
    $n = Get-Command node -ErrorAction SilentlyContinue
    if ($n) { return @{ Exe = $n.Source; Electron = $false } }
    throw "no usable Node runtime found (looked for VS Code's Code.exe, then node on PATH)"
}
$node = Get-NodeRunner

function Invoke-Child([string]$exePath, [string]$argLine, [string]$workDir, [hashtable]$envVars) {
    # Start-Process/& both mangle stderr here; use ProcessStartInfo directly.
    # See vm-pitfalls section on PowerShell child-process traps.
    $psi = New-Object System.Diagnostics.ProcessStartInfo
    $psi.FileName               = $exePath
    $psi.Arguments              = $argLine
    $psi.WorkingDirectory       = $workDir
    $psi.RedirectStandardOutput = $true
    $psi.RedirectStandardError  = $true
    $psi.UseShellExecute        = $false
    $psi.CreateNoWindow         = $true
    if ($envVars) { foreach ($k in $envVars.Keys) { $psi.EnvironmentVariables[$k] = $envVars[$k] } }

    $p = New-Object System.Diagnostics.Process
    $p.StartInfo = $psi
    [void]$p.Start()
    $oTask = $p.StandardOutput.ReadToEndAsync()
    $eTask = $p.StandardError.ReadToEndAsync()
    if (-not $p.WaitForExit($TimeoutSec * 1000)) {
        try { $p.Kill() } catch {}
        return @{ TimedOut = $true; Out = ''; Err = ''; Code = -1 }
    }
    return @{ TimedOut = $false; Out = $oTask.Result; Err = $eTask.Result; Code = $p.ExitCode }
}

# Pull "line:col" keys out of arrow.exe's static-error table. Messages wrap across
# terminal columns, so positions are the reliable key; '<unknown>' rows are counted.
# Only rows of $mainFile: the editor shows a document's own errors (an imported module's errors
# are shown when that file is open).
function Get-ExeErrorKeys([string]$text, [string]$mainFile) {
    $clean = $text -replace "$([char]27)\[[0-9;]*m", ''
    $main = ($mainFile -replace '\\', '/').ToLowerInvariant()
    $keys = New-Object System.Collections.Generic.List[string]
    foreach ($line in ($clean -split "`r?`n")) {
        if ($line -match '^(\S.*?)\s+(\d+):(\d+)\s+StaticTypeError\s') {
            $file = ($Matches[1].Trim() -replace '\\', '/').ToLowerInvariant()
            if ($file -ne $main) { continue }
            $keys.Add("$($Matches[2]):$($Matches[3])")
        } elseif ($line -match '^<unknown>\s+-\s+StaticTypeError\s') {
            $keys.Add('<unknown>')
        }
    }
    # D5 (task 5-0): the editor now expands metafunctions too, so a failed expansion is
    # reported by both sides as a MetaError. arrow.exe prints it as text, not in the table:
    # the key is the first 'while expanding ... at <file>:<line>:<col>' (the statement in
    # this file that started the expansion), or '<unknown>' when there is no position.
    # A template instantiation that fails at expansion time (a constraint violation) is printed
    # as 'TemplateError:' -- the same kind of stop (3-3: the editor used to drop it silently).
    if ($clean -match '(?m)^(MetaError|TemplateError):') {
        $at = ($clean -split "`r?`n") | Where-Object { $_ -match "^\s*while expanding '" } | Select-Object -First 1
        if ($at -and $at -match ':(\d+):(\d+)\s*$') { $keys.Add("$($Matches[1]):$($Matches[2])") }
        elseif ($at -and $at -match 'line (\d+), col (\d+)\s*$') { $keys.Add("$($Matches[1]):$($Matches[2])") }
        else { $keys.Add('<unknown>') }
    }
    return $keys
}

# A ParseError message without its trailing location, with paths in one form. The wasm passes
# Windows paths as '/D:/a/b' (vscode-extension/src/wasm_host.ts) and arrow.exe prints 'D:\a\b'.
function Normalize-ParseMessage([string]$msg) {
    $m = ($msg -replace '^(ParseError:\s*)+', '').Trim()
    # CLI only: ' at file:line:col' at the end (the LAST ' at ', see the parse-error branch below).
    $m = ($m -replace '^(.*)\s+at\s+.+?:\d+:\d+$', '$1').Trim()
    $m = $m -replace '\\', '/'
    $m = $m -replace "(^|[\s'`"(])/([A-Za-z]):(?:/|(?=['`"\s),]|$))", '$1$2:/'
    return $m
}

$files = Get-ChildItem -Path (Join-Path $repo 'examples') -Recurse -Filter '*.ar' |
    Where-Object { $_.FullName -notlike '*\archived\*' } |
    Sort-Object FullName

Write-Host ''
Write-Host 'compare_wasm_frontend -- arrow.exe vs wasm editor frontend' -ForegroundColor Cyan
Write-Host ('-' * 78)

$checked = 0; $agreed = 0; $missed = 0; $invented = 0; $parseFail = 0; $skipped = 0
$inventedList = New-Object System.Collections.Generic.List[string]
$missedList = New-Object System.Collections.Generic.List[string]
# arrow.exe stops after the static checks (src/main.rs) -- the examples are never executed.
$checkOnly = @{ AR_CHECK_ONLY = '1' }
$parseFailList = New-Object System.Collections.Generic.List[string]

foreach ($f in $files) {
    $rel = $f.FullName.Substring($repo.Length + 1)

    # --- wasm side ---
    $args = '"{0}" "{1}" "{2}"' -f $dump, $wasm, $f.FullName
    $envv = if ($node.Electron) { @{ ELECTRON_RUN_AS_NODE = '1' } } else { $null }
    $w = Invoke-Child $node.Exe $args $repo $envv
    if ($w.TimedOut) { Write-Host ("TIMEOUT(wasm) {0}" -f $rel) -ForegroundColor Red; $skipped++; continue }

    $wj = $null
    try { $wj = $w.Out | ConvertFrom-Json } catch {
        Write-Host ("BAD-JSON     {0}  {1}" -f $rel, $w.Err.Trim()) -ForegroundColor Red
        $skipped++; continue
    }

    if (-not $wj.ok) {
        # The editor frontend could not parse it. That is only correct if arrow.exe
        # cannot parse it either -- otherwise the editor rejects code the compiler
        # accepts, which is just as bad as inventing type errors.
        $e = Invoke-Child $exe ('-src "{0}"' -f $f.FullName) $f.DirectoryName $checkOnly
        if ($e.TimedOut) { Write-Host ("TIMEOUT(exe)  {0}" -f $rel) -ForegroundColor Yellow; $skipped++; continue }
        $exeText = (($e.Out + "`n" + $e.Err) -replace "$([char]27)\[[0-9;]*m", '')
        if ($exeText -match 'ParseError:') {
            # Both reject it. Compare the message tail so a divergence in *why*
            # still shows up.
            $exeMsg  = ((($exeText -split "`r?`n") | Where-Object { $_ -match 'ParseError:' } | Select-Object -First 1) -replace '^(ParseError:\s*)+', '').Trim()
            # CLI だけが末尾に位置（` at file:line:col`）を付ける（フェーズ10 10-17・エディタは位置を別に持つ）。
            # ⚠ **最後の** ` at ` から落とす。文面そのものに ` at ` を含むことがある
            #   （`must be at the top level`・10-16。最初の ` at ` から落として文面が切れていた）。
            $exeMsg  = ($exeMsg -replace '^(.*)\s+at\s+.+?:\d+:\d+$', '$1').Trim()
            $wasmMsg = ($wj.parseError -replace '^(ParseError:\s*)+', '').Trim()
            if ($exeMsg -eq $wasmMsg) {
                $checked++; $agreed++
                if ($VerboseDiff) { Write-Host ("agree(parse) {0}" -f $rel) -ForegroundColor DarkGray }
            } else {
                $checked++; $parseFail++
                $parseFailList.Add("$rel  MESSAGE DIFFERS`n      exe : $exeMsg`n      wasm: $wasmMsg")
                Write-Host ("PARSE-DIFF   {0}" -f $rel) -ForegroundColor Red
            }
        } else {
            # arrow.exe accepted it, the editor did not: a real regression.
            $checked++; $parseFail++
            $parseFailList.Add("$rel  REJECTED BY EDITOR ONLY  --  $($wj.parseError)")
            Write-Host ("EDITOR-ONLY  {0}  {1}" -f $rel, $wj.parseError) -ForegroundColor Red
        }
        continue
    }

    $wasmKeys = @()
    foreach ($d in $wj.diagnostics) {
        if ($d.severity -ne 0) { continue }
        if ($null -eq $d.at) { $wasmKeys += '<unknown>' }
        else { $wasmKeys += "$($d.at.line + 1):$($d.at.col + 1)" }
    }

    # arrow.exe runs on every file (check only), so a file the editor finds clean is compared too.
    $e = Invoke-Child $exe ('-src "{0}"' -f $f.FullName) $f.DirectoryName $checkOnly
    if ($e.TimedOut) { Write-Host ("TIMEOUT(exe)  {0}" -f $rel) -ForegroundColor Yellow; $skipped++; continue }

    # An import arrow.exe could not resolve: it stops with a ParseError. The editor does not stop
    # (editor_import_resolution_plan.md 3-2): it reports the import as a 'ParseError' diagnostic and
    # analyses the rest of the file. So the first such diagnostic must say what arrow.exe said; the
    # editor's other diagnostics cannot be checked (arrow.exe never got that far).
    $exeText = (($e.Out + "`n" + $e.Err) -replace "$([char]27)\[[0-9;]*m", '')
    if ($exeText -match '(?m)^ParseError:') {
        $checked++
        $exeMsg = Normalize-ParseMessage ((($exeText -split "`r?`n") | Where-Object { $_ -match '^ParseError:' } | Select-Object -First 1))
        $wasmImport = @($wj.diagnostics | Where-Object { $_.source -eq 'ParseError' }) | Select-Object -First 1
        if ($null -eq $wasmImport) {
            $parseFail++
            $parseFailList.Add("$rel  REJECTED BY arrow.exe ONLY  --  $exeMsg")
            Write-Host ("EXE-ONLY     {0}" -f $rel) -ForegroundColor Red
        } elseif ((Normalize-ParseMessage $wasmImport.message) -eq $exeMsg) {
            $agreed++
            if ($VerboseDiff) { Write-Host ("agree(import) {0}" -f $rel) -ForegroundColor DarkGray }
        } else {
            $parseFail++
            $parseFailList.Add("$rel  MESSAGE DIFFERS`n      exe : $exeMsg`n      wasm: $(Normalize-ParseMessage $wasmImport.message)")
            Write-Host ("PARSE-DIFF   {0}" -f $rel) -ForegroundColor Red
        }
        continue
    }
    $exeKeys = Get-ExeErrorKeys ($e.Out + "`n" + $e.Err) $f.FullName

    $checked++
    $extra = @($wasmKeys | Where-Object { $_ -notin $exeKeys })
    $missing = @($exeKeys | Where-Object { $_ -notin $wasmKeys })

    if ($extra.Count -gt 0) {
        $invented++
        $inventedList.Add("$rel  invented=[$($extra -join ', ')]")
        Write-Host ("INVENTED     {0}  wasm-only=[{1}]" -f $rel, ($extra -join ', ')) -ForegroundColor Red
    }
    if ($missing.Count -gt 0) {
        $missed++
        $missedList.Add("$rel  missed=[$($missing -join ', ')]")
        Write-Host ("MISSED       {0}  exe-only=[{1}]" -f $rel, ($missing -join ', ')) -ForegroundColor Red
    }
    if ($extra.Count -eq 0 -and $missing.Count -eq 0) {
        $agreed++
        if ($VerboseDiff) { Write-Host ("agree        {0}  ({1} errors)" -f $rel, $wasmKeys.Count) -ForegroundColor DarkGray }
    }
}

Write-Host ('-' * 78)
Write-Host ("compared      : {0}" -f $checked)
Write-Host ("agreed        : {0}" -f $agreed) -ForegroundColor Green
Write-Host ("wasm MISSED   : {0}   <- must be 0" -f $missed) -ForegroundColor $(if ($missed -eq 0) { 'Green' } else { 'Red' })
Write-Host ("wasm INVENTED : {0}   <- must be 0" -f $invented) -ForegroundColor $(if ($invented -eq 0) { 'Green' } else { 'Red' })
Write-Host ("parse mismatch: {0}   <- must be 0" -f $parseFail) -ForegroundColor $(if ($parseFail -eq 0) { 'Green' } else { 'Red' })
Write-Host ("skipped       : {0}" -f $skipped)

if ($parseFailList.Count -gt 0) {
    Write-Host ''
    Write-Host 'PARSE DISAGREEMENTS (editor and arrow.exe do not match):' -ForegroundColor Red
    foreach ($x in $parseFailList) { Write-Host "  $x" }
}
if ($inventedList.Count -gt 0) {
    Write-Host ''
    Write-Host 'INVENTED ERRORS (these are false positives in the editor):' -ForegroundColor Red
    foreach ($x in $inventedList) { Write-Host "  $x" }
}
if ($missedList.Count -gt 0) {
    Write-Host ''
    Write-Host 'MISSED ERRORS (arrow.exe reports them, the editor does not):' -ForegroundColor Red
    foreach ($x in $missedList) { Write-Host "  $x" }
}
if ($invented -gt 0 -or $missed -gt 0 -or $parseFail -gt 0) { exit 1 }
exit 0
