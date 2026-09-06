# run_extension_debug.ps1 -- run the VS Code extension's standalone debug runner on a .ar file.
#
# Why this exists
# ---------------
# `node vscode-extension/run_debug.js <file.ar>` is the way to exercise hover / inlay /
# semantic tokens / completion without launching VS Code (skill `vscode-debug-runner`).
# But the `node` on PATH here is v11, which **cannot compile the frontend wasm**
# (`CompileError: expected table index 0, found 128`).
#
# compare_wasm_frontend.ps1 already solved this: VS Code ships a modern Node, and any
# machine that runs this extension has it. This script reuses the same discovery so the
# debug runner works regardless of what `node` on PATH happens to be.
#
# Usage:
#   ./scripts/run_extension_debug.ps1 examples/basics/variable.ar
#   ./scripts/run_extension_debug.ps1 <file.ar> -Build     # recompile TS first
#
# ⚠ The runner reads `vscode-extension/out_debug/`, which is built by
#   `npx tsc -p tsconfig.debug.json`. Pass -Build after editing any .ts file, or the
#   run silently exercises the PREVIOUS version of the providers.

param(
    [Parameter(Mandatory = $true, Position = 0)] [string] $File,
    [switch] $Build
)

$ErrorActionPreference = 'Stop'

$repo = Split-Path (Split-Path $MyInvocation.MyCommand.Path)
$ext  = Join-Path $repo 'vscode-extension'

if (-not (Test-Path $File)) { throw "not found: $File" }

# --- locate a Node that understands modern wasm opcodes (same as compare_wasm_frontend.ps1)
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

if ($Build) {
    Write-Host 'Compiling out_debug ...' -ForegroundColor Cyan
    Push-Location $ext
    try {
        # PS 5.1 trap: a native exe writing to stderr becomes a NativeCommandError and
        # $ErrorActionPreference='Stop' turns that into a terminating error even on exit 0.
        $prevEAP = $ErrorActionPreference
        $ErrorActionPreference = 'Continue'
        npx tsc -p tsconfig.debug.json 2>&1 | ForEach-Object { Write-Host "  $_" }
        $exit = $LASTEXITCODE
        $ErrorActionPreference = $prevEAP
        if ($exit -ne 0) { throw "tsc failed (exit $exit)" }
    } finally { Pop-Location }
}

$runner = Join-Path $ext 'run_debug.js'
$target = (Resolve-Path $File).Path

# ⚠ `&` も Start-Process も、Electron を Node として起動したときの stdout を取りこぼす。
#    compare_wasm_frontend.ps1 と同じく ProcessStartInfo を直に使う
#    （skill `vm-pitfalls` の PowerShell 子プロセスの節）。
$psi = New-Object System.Diagnostics.ProcessStartInfo
$psi.FileName               = $node.Exe
$psi.Arguments              = '"{0}" "{1}"' -f $runner, $target
$psi.WorkingDirectory       = $repo
$psi.RedirectStandardOutput = $true
$psi.RedirectStandardError  = $true
$psi.UseShellExecute        = $false
$psi.CreateNoWindow         = $true
if ($node.Electron) { $psi.EnvironmentVariables['ELECTRON_RUN_AS_NODE'] = '1' }

$p = New-Object System.Diagnostics.Process
$p.StartInfo = $psi
[void]$p.Start()
$oTask = $p.StandardOutput.ReadToEndAsync()
$eTask = $p.StandardError.ReadToEndAsync()
if (-not $p.WaitForExit(120000)) {
    try { $p.Kill() } catch {}
    throw "run_debug.js timed out after 120s"
}
Write-Host $oTask.Result
$err = $eTask.Result
if ($err.Trim()) { Write-Host $err -ForegroundColor Red }
exit $p.ExitCode
