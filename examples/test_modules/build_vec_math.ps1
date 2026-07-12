# build_vec_math.ps1 — build vec_math_x64.lib from vec_math.c with MSVC.
# The `_x64.lib` suffix matches the cpp bridge's default lib_patterns so the
# shim links it automatically (src/interpreter/cpp_bridge/compiler.rs).
# vcvarsall.bat is read from ar_config.json (cpp.msvc); falls back to VS2022/18.
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path

# Resolve vcvarsall.bat from ar_config.json at the repo root
$repoRoot = (Resolve-Path (Join-Path $here '..\..')).Path
$vcvarsall = $null
$cfgPath = Join-Path $repoRoot 'ar_config.json'
if (Test-Path $cfgPath) {
    $cfg = Get-Content $cfgPath -Raw | ConvertFrom-Json
    if ($cfg.cpp -and $cfg.cpp.msvc) { $vcvarsall = $cfg.cpp.msvc }
}
if (-not $vcvarsall -or -not (Test-Path $vcvarsall)) {
    Write-Error "vcvarsall.bat not found (set cpp.msvc in ar_config.json)"
}

$bat = @"
@echo off
call "$vcvarsall" x64
cd /d "$here"
rem /MD is required: the cpp-bridge shim links with /MD /NODEFAULTLIB:LIBCMT,
rem so an /MT-built (default) lib would leave CRT symbols unresolved.
cl /nologo /c /O2 /MD vec_math.c
if errorlevel 1 exit /b 1
lib /nologo /OUT:vec_math_x64.lib vec_math.obj
if errorlevel 1 exit /b 1
del vec_math.obj
"@
$batFile = Join-Path $env:TEMP 'build_vec_math.bat'
Set-Content -Path $batFile -Value $bat -Encoding ascii
& cmd /c $batFile
if ($LASTEXITCODE -ne 0) { Write-Error "build failed (exit $LASTEXITCODE)" }
Write-Host "OK: $(Join-Path $here 'vec_math_x64.lib')"
