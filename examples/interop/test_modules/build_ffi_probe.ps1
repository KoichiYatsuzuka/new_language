# build_ffi_probe.ps1 — ffi_probe.c から 2 つのライブラリを作る（ffi_value_exchange_plan.md 1-1）。
#
#   ffi_probe.dll      … import[cpp-dll] 用（ヘッダの隣の {stem}.dll を読む）
#   ffi_probe_x64.lib  … import[cpp-lib] 用（既定の lib_patterns が `_x64.lib` を拾う）
#
# - DLL は ffi_probe.def で export する（ソースに __declspec(dllexport) を書かない。静的ライブラリ側にも
#   付くと、cpp-lib のシムが `/EXPORT:name=ar_shim_name` で同じ名前を export して衝突する）
# - DLL の import ライブラリ（ffi_probe.lib）は作らない（/NOIMPLIB）。cpp-lib の探索に紛れないように
# - /MD 必須（cpp-lib のシムは /MD /NODEFAULTLIB:LIBCMT でリンクする。build_vec_math.ps1 と同じ）
# - /utf-8: ソースは ASCII だが、念のため CP932 で読ませない
#
# vcvarsall.bat はリポジトリ直下の ar_config.json（cpp.msvc）から読む。
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path

$repoRoot = (Resolve-Path (Join-Path $here '..\..\..')).Path
$vcvarsall = $null
$cfgPath = Join-Path $repoRoot 'ar_config.json'
if (Test-Path $cfgPath) {
    $cfg = Get-Content $cfgPath -Raw | ConvertFrom-Json
    if ($cfg.cpp -and $cfg.cpp.msvc) { $vcvarsall = $cfg.cpp.msvc }
}
if (-not $vcvarsall -or -not (Test-Path $vcvarsall)) {
    Write-Error "vcvarsall.bat not found (set cpp.msvc in ar_config.json)"
}

# vcvarsall は vswhere.exe を呼ぶ。PATH に無いと stderr に出し、Stop がそれを終了扱いにするので足しておく
# （build_bridge.ps1 と同じ）。出力も捨てる。
$env:PATH = "C:\Program Files (x86)\Microsoft Visual Studio\Installer;$env:PATH"

$bat = @"
@echo off
call "$vcvarsall" x64 >nul 2>nul
cd /d "$here"
cl /nologo /utf-8 /O2 /MD /LD ffi_probe.c /link /DEF:ffi_probe.def /NOIMPLIB /OUT:ffi_probe.dll
if errorlevel 1 exit /b 1
del ffi_probe.obj ffi_probe.exp 2>nul
cl /nologo /utf-8 /c /O2 /MD ffi_probe.c
if errorlevel 1 exit /b 1
lib /nologo /OUT:ffi_probe_x64.lib ffi_probe.obj
if errorlevel 1 exit /b 1
del ffi_probe.obj
"@
$batFile = Join-Path $env:TEMP 'build_ffi_probe.bat'
Set-Content -Path $batFile -Value $bat -Encoding ascii
& cmd /c $batFile
if ($LASTEXITCODE -ne 0) { Write-Error "build failed (exit $LASTEXITCODE)" }
Write-Host "OK: $(Join-Path $here 'ffi_probe.dll')"
Write-Host "OK: $(Join-Path $here 'ffi_probe_x64.lib')"
