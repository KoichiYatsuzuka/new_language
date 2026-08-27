# compare_wasm_frontend.ps1 -- verify the wasm editor frontend agrees with arrow.exe.
#
# The VS Code extension analyses .ar files with crates/arrow-frontend compiled to wasm.
# That build differs from the shipping binary in exactly one way: `editor` feature
# replaces parse-time module loading with a syntax-only stub (see
# src/parser/imports_editor.rs). So the required property is NOT "identical output",
# it is:
#
#   * import-free example  -> the two must report the SAME static type errors
#   * example with imports -> wasm may report FEWER (module members are unknown),
#                             but must NEVER report an error arrow.exe does not have
#
# The second half is the one that matters: an editor that invents errors is exactly
# the bug this whole change is meant to remove.
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
function Get-ExeErrorKeys([string]$text) {
    $clean = $text -replace "$([char]27)\[[0-9;]*m", ''
    $keys = New-Object System.Collections.Generic.List[string]
    foreach ($line in ($clean -split "`r?`n")) {
        if ($line -match '\s(\d+):(\d+)\s+StaticTypeError\s') {
            $keys.Add("$($Matches[1]):$($Matches[2])")
        } elseif ($line -match '^<unknown>\s+-\s+StaticTypeError\s') {
            $keys.Add('<unknown>')
        }
    }
    return $keys
}

$files = Get-ChildItem -Path (Join-Path $repo 'examples') -Recurse -Filter '*.ar' |
    Where-Object { $_.FullName -notlike '*\archived\*' } |
    Sort-Object FullName

Write-Host ''
Write-Host 'compare_wasm_frontend -- arrow.exe vs wasm editor frontend' -ForegroundColor Cyan
Write-Host ('-' * 78)

$checked = 0; $agreed = 0; $fewer = 0; $invented = 0; $parseFail = 0; $skipped = 0
$inventedList = New-Object System.Collections.Generic.List[string]
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
        $e = Invoke-Child $exe ('-src "{0}"' -f $f.FullName) $f.DirectoryName $null
        if ($e.TimedOut) { Write-Host ("TIMEOUT(exe)  {0}" -f $rel) -ForegroundColor Yellow; $skipped++; continue }
        $exeText = (($e.Out + "`n" + $e.Err) -replace "$([char]27)\[[0-9;]*m", '')
        if ($exeText -match 'ParseError:') {
            # Both reject it. Compare the message tail so a divergence in *why*
            # still shows up.
            $exeMsg  = ((($exeText -split "`r?`n") | Where-Object { $_ -match 'ParseError:' } | Select-Object -First 1) -replace '^(ParseError:\s*)+', '').Trim()
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

    # Only run arrow.exe when there is something to compare against: any file where
    # either side reports errors. Clean files are skipped because running them would
    # execute the example (GUI, FFI, sleeps).
    if ($wasmKeys.Count -eq 0) { $checked++; $agreed++; continue }

    $e = Invoke-Child $exe ('-src "{0}"' -f $f.FullName) $f.DirectoryName $null
    if ($e.TimedOut) { Write-Host ("TIMEOUT(exe)  {0}" -f $rel) -ForegroundColor Yellow; $skipped++; continue }
    $exeKeys = Get-ExeErrorKeys ($e.Out + "`n" + $e.Err)

    $checked++
    $extra = @($wasmKeys | Where-Object { $_ -notin $exeKeys })
    $missing = @($exeKeys | Where-Object { $_ -notin $wasmKeys })

    if ($extra.Count -gt 0) {
        $invented++
        $inventedList.Add("$rel  invented=[$($extra -join ', ')]")
        Write-Host ("INVENTED     {0}  wasm-only=[{1}]" -f $rel, ($extra -join ', ')) -ForegroundColor Red
    } elseif ($missing.Count -gt 0) {
        $fewer++
        if ($VerboseDiff) {
            Write-Host ("fewer        {0}  exe-only=[{1}]" -f $rel, ($missing -join ', ')) -ForegroundColor DarkYellow
        }
    } else {
        $agreed++
        if ($VerboseDiff) { Write-Host ("agree        {0}  ({1} errors)" -f $rel, $wasmKeys.Count) -ForegroundColor DarkGray }
    }
}

Write-Host ('-' * 78)
Write-Host ("compared      : {0}" -f $checked)
Write-Host ("agreed        : {0}" -f $agreed) -ForegroundColor Green
Write-Host ("wasm fewer    : {0}   (expected for files with imports)" -f $fewer) -ForegroundColor DarkYellow
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
if ($invented -gt 0 -or $parseFail -gt 0) { exit 1 }
exit 0
