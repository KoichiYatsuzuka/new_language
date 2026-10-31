# compare_tm_grammar.ps1 -- verify the extension's TextMate grammar agrees with the real lexer.
#
# The VS Code extension colours identifiers from the wasm lexer, but strings, comments and bracket
# matching come ONLY from vscode-extension/syntaxes/arrow.tmLanguage.json, a regex-based second
# lexer. This gate tokenizes every example with both and compares, character by character, whether
# it is inside a string, inside a comment, or neither (scripts/compare_tm_grammar.js has the details).
#
#   * SPILL must be 0: the grammar carried a string/comment from one line into code on a later line
#     (brackets stop matching, comments turn white -- VSIX_update_plan.md).
#   * The grammar's keyword list must match src/lexer/keyword.rs.
#   * Other differences inside a line are reported only (-VerboseDiff lists them).
#
# Population: examples/**/*.ar except examples/archived (the same as compare_wasm_frontend.ps1),
# plus vscode-extension/test_fixtures/*.ar (the edge cases of the plan).
#
# The TextMate engine is the one VS Code itself uses, taken from the installed VS Code
# (resources/app/node_modules.asar) into target/tm_engine/<version>/ on first use, and run with
# VS Code's Electron as Node (the node on PATH may be too old for it).
#
# ASCII-only on purpose: PowerShell 5.1 reads BOM-less .ps1 as ANSI.

param(
    [int]$TimeoutSec = 600,
    [switch]$VerboseDiff,
    [switch]$SkipCompile,
    # Files to check instead of the whole population.
    [string[]]$Files
)

$ErrorActionPreference = 'Stop'
$repo    = Split-Path -Parent $PSScriptRoot
$wasm    = Join-Path $repo 'crates\arrow-frontend\target\wasm32-unknown-unknown\release\arrow_frontend.wasm'
$grammar = Join-Path $repo 'vscode-extension\syntaxes\arrow.tmLanguage.json'
$keyword = Join-Path $repo 'src\lexer\keyword.rs'
$helper  = Join-Path $PSScriptRoot 'compare_tm_grammar.js'

if (-not (Test-Path $wasm)) { throw "not built: $wasm (cd crates/arrow-frontend; cargo build --release --target wasm32-unknown-unknown)" }

# --- VS Code: Electron (as Node) and its TextMate engine --------------------------
$codeExe = @(
    "$env:LOCALAPPDATA\Programs\Microsoft VS Code\Code.exe",
    "$env:ProgramFiles\Microsoft VS Code\Code.exe"
) | Where-Object { Test-Path $_ } | Select-Object -First 1
if (-not $codeExe) { throw 'VS Code (Code.exe) not found: it provides both the TextMate engine and a recent Node' }
$codeDir = Split-Path -Parent $codeExe
# Newer installs keep resources under a per-version directory (<commit>\resources\app).
$asar = Get-ChildItem -Path $codeDir -Recurse -Depth 3 -Filter 'node_modules.asar' -ErrorAction SilentlyContinue |
    Where-Object { $_.FullName -like '*\resources\app\node_modules.asar' } |
    Sort-Object LastWriteTime -Descending | Select-Object -First 1
if (-not $asar) { throw "node_modules.asar not found under $codeDir" }
$version = Split-Path -Leaf (Split-Path -Parent (Split-Path -Parent (Split-Path -Parent $asar.FullName)))
$engine  = Join-Path $repo "target\tm_engine\$version"

function Invoke-Electron([string]$argLine) {
    $psi = New-Object System.Diagnostics.ProcessStartInfo
    $psi.FileName = $codeExe
    $psi.Arguments = $argLine
    $psi.WorkingDirectory = $repo
    $psi.RedirectStandardOutput = $true
    $psi.RedirectStandardError = $true
    $psi.UseShellExecute = $false
    $psi.CreateNoWindow = $true
    $psi.EnvironmentVariables['ELECTRON_RUN_AS_NODE'] = '1'
    $psi.StandardOutputEncoding = [System.Text.Encoding]::UTF8
    $psi.StandardErrorEncoding = [System.Text.Encoding]::UTF8
    $p = New-Object System.Diagnostics.Process
    $p.StartInfo = $psi
    [void]$p.Start()
    $oTask = $p.StandardOutput.ReadToEndAsync()
    $eTask = $p.StandardError.ReadToEndAsync()
    if (-not $p.WaitForExit($TimeoutSec * 1000)) {
        & taskkill /T /F /PID $p.Id 2>&1 | Out-Null
        Write-Host ("ABNORMAL TERMINATION: compare_tm_grammar.js did not finish in {0} s (killed)" -f $TimeoutSec) -ForegroundColor Red
        exit 2
    }
    $p.WaitForExit()
    return @{ Out = $oTask.Result; Err = $eTask.Result; Code = $p.ExitCode }
}

if (-not (Test-Path (Join-Path $engine 'vscode-textmate'))) {
    Write-Host "extracting the TextMate engine of VS Code $version ..." -ForegroundColor Cyan
    $r = Invoke-Electron ('"{0}" --extract "{1}" "{2}"' -f $helper, $asar.FullName, $engine)
    if ($r.Code -ne 0) { Write-Host $r.Out; Write-Host $r.Err -ForegroundColor Red; throw 'engine extraction failed' }
}

# --- the extension's host (frontend.js loads the wasm) -------------------------------
if (-not $SkipCompile) {
    Write-Host 'compiling vscode-extension (tsc) ...' -ForegroundColor Cyan
    $tsc = & npm --prefix (Join-Path $repo 'vscode-extension') run compile 2>&1
    if ($LASTEXITCODE -ne 0) {
        $tsc | ForEach-Object { Write-Host $_ }
        Write-Host 'EXTENSION-COMPILE: FAILED' -ForegroundColor Red
        exit 1
    }
}

# --- population ------------------------------------------------------------------
if ($Files) {
    $list = $Files | ForEach-Object { (Resolve-Path $_).Path }
} else {
    $list = @(Get-ChildItem -Path (Join-Path $repo 'examples') -Recurse -Filter '*.ar' |
        Where-Object { $_.FullName -notlike '*\archived\*' } | Sort-Object FullName | ForEach-Object { $_.FullName })
    $fixtures = Join-Path $repo 'vscode-extension\test_fixtures'
    if (Test-Path $fixtures) {
        $list += @(Get-ChildItem -Path $fixtures -Filter '*.ar' | Sort-Object FullName | ForEach-Object { $_.FullName })
    }
}
$listFile = Join-Path ([System.IO.Path]::GetTempPath()) ("tm_grammar_files_{0}.txt" -f $PID)
[System.IO.File]::WriteAllLines($listFile, [string[]]$list, (New-Object System.Text.UTF8Encoding($false)))

Write-Host ''
Write-Host 'compare_tm_grammar -- TextMate grammar vs the lexer (wasm)' -ForegroundColor Cyan
Write-Host ('-' * 78)
$verboseArg = if ($VerboseDiff) { ' --verbose' } else { '' }
$r = Invoke-Electron ('"{0}" --engine "{1}" --grammar "{2}" --wasm "{3}" --keywords "{4}" --list "{5}"{6}' -f `
    $helper, $engine, $grammar, $wasm, $keyword, $listFile, $verboseArg)
Remove-Item $listFile -ErrorAction SilentlyContinue
Write-Host $r.Out
if ($r.Err.Trim()) { Write-Host $r.Err -ForegroundColor Red }
exit $r.Code
