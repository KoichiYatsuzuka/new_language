# gen_builtin_py_stubs.ps1 - regenerate the bundled type stubs of Python modules that have no .py
# (stubs/generated/*.pyi and src/py_stubs_generated.rs; pandas_import_plan.md tasks 1-4 / 1-6).
# Run from the repository root after changing the CPython version or the typeshed copy:
#   ./scripts/gen_builtin_py_stubs.ps1 [-Typeshed <dir with stdlib\ and LICENSE>]
# Modules typeshed has are made from typeshed (default: the copy Pylance bundles, typeshed-fallback); the others by
# CPython introspection. The work is done by scripts/gen_builtin_py_stubs.py (and typeshed_flatten.py).
param([string]$Typeshed = '')
$ErrorActionPreference = 'Stop'
$python = (Get-Command python -ErrorAction Stop).Source
$pyArgs = @((Join-Path $PSScriptRoot 'gen_builtin_py_stubs.py'))
if ($Typeshed -ne '') { $pyArgs += @('--typeshed', $Typeshed) }
& $python @pyArgs
if ($LASTEXITCODE -ne 0) { throw "gen_builtin_py_stubs.py failed (exit $LASTEXITCODE)" }
Write-Host "Rebuild arrow (cargo build --release) and the wasm frontend to pick up the stubs." -ForegroundColor Cyan
