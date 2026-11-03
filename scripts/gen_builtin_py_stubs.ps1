# gen_builtin_py_stubs.ps1 - regenerate the bundled type stubs of Python modules that have no .py
# (stubs/generated/*.pyi and src/py_stubs_generated.rs; pandas_import_plan.md task 1-4).
# Run from the repository root after changing the CPython version: ./scripts/gen_builtin_py_stubs.ps1
# The work is done by scripts/gen_builtin_py_stubs.py (CPython introspection).
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$python = (Get-Command python -ErrorAction Stop).Source
& $python (Join-Path $PSScriptRoot 'gen_builtin_py_stubs.py')
if ($LASTEXITCODE -ne 0) { throw "gen_builtin_py_stubs.py failed (exit $LASTEXITCODE)" }
Write-Host "Rebuild arrow (cargo build --release) and the wasm frontend to pick up the stubs." -ForegroundColor Cyan
