# check_ptr_protocol.ps1 — PtrProtocol の C ヘッダと C# の写しが、Arrow（Rust）の写しと同じ並びかを実際にコンパイルして確かめる
# （ffi_value_exchange_plan.md 4-1）。
#
#   - bridge/ptr_protocol/ar_ptr_protocol.h を MSVC で C と C++ の両方としてコンパイルし、大きさ・欄の位置・flags を出す
#   - bridge/ptr_protocol/ArPtrProtocol.cs を dotnet でビルドし、Marshal.SizeOf / OffsetOf を出す
#   - どちらも Rust の写し（src/interpreter/value/ptr.rs の PtrProtocolRaw）の期待値と比べる
#
# Rust 側の定数・欄の順序は単体テスト（protocol_files_agree / protocol_layout_matches_the_header）が見る。このスクリプトは
# 「C と C# のコンパイラが実際にその並びにするか」を見る（ツールチェーンが要るので単体テストにはしない）。
#
# 使い方: ./scripts/check_ptr_protocol.ps1      （vcvarsall.bat は ar_config.json の cpp.msvc・dotnet は PATH）
# ⚠ このファイルは日本語コメントを含むので UTF-8 BOM 付きで保存すること。
param([int]$TimeoutMin = 10)
$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
$proto = Join-Path $repo 'bridge\ptr_protocol'
$work = Join-Path $env:TEMP 'ar_check_ptr_protocol'
if (Test-Path $work) { Remove-Item -Recurse -Force $work }
New-Item -ItemType Directory -Force $work | Out-Null

# Rust の写しの期待値（PtrProtocolRaw）
$expected = 'size=48 address=0 elem_type=8 flags=16 _reserved=20 length=24 release=32 release_ctx=40 CONST=1 NUL=2 STATIC=4'

function Invoke-Limited([string]$file, [string[]]$argv, [string]$what) {
    $out = Join-Path $work "$what.out.txt"
    $opts = @{ FilePath = $file; NoNewWindow = $true; PassThru = $true; WorkingDirectory = $work
               RedirectStandardOutput = $out; RedirectStandardError = "$out.err" }
    # ⚠ Start-Process は空の ArgumentList を受け付けない
    if ($argv.Count -gt 0) { $opts.ArgumentList = $argv }
    $p = Start-Process @opts
    # ⚠ PowerShell 5.1 は起動直後にハンドルを取っておかないと、終わった後の ExitCode が空になる
    $null = $p.Handle
    if (-not $p.WaitForExit($TimeoutMin * 60 * 1000)) {
        & taskkill.exe /T /F /PID $p.Id 2>&1 | Out-Null
        Write-Host "ABNORMAL TERMINATION: $what exceeded $TimeoutMin min and was killed" -ForegroundColor Red
        exit 2
    }
    $p.WaitForExit()
    if ($p.ExitCode -ne 0) {
        Get-Content $out, "$out.err" -ErrorAction SilentlyContinue | Write-Host
        Write-Host "FAILED: $what (exit $($p.ExitCode))" -ForegroundColor Red
        exit 1
    }
    return (Get-Content $out -Raw)
}

# ── C / C++ ──────────────────────────────────────────────────────────────────
$cfg = Get-Content (Join-Path $repo 'ar_config.json') -Raw | ConvertFrom-Json
$vcvarsall = $cfg.cpp.msvc
if (-not $vcvarsall -or -not (Test-Path $vcvarsall)) { throw 'vcvarsall.bat not found (set cpp.msvc in ar_config.json)' }
$c = @'
#include <stdio.h>
#include <stddef.h>
#include "ar_ptr_protocol.h"
int main(void) {
    printf("size=%u address=%u elem_type=%u flags=%u _reserved=%u length=%u release=%u release_ctx=%u CONST=%u NUL=%u STATIC=%u",
        (unsigned)sizeof(ArPtrProtocol), (unsigned)offsetof(ArPtrProtocol, address),
        (unsigned)offsetof(ArPtrProtocol, elem_type), (unsigned)offsetof(ArPtrProtocol, flags),
        (unsigned)offsetof(ArPtrProtocol, _reserved), (unsigned)offsetof(ArPtrProtocol, length),
        (unsigned)offsetof(ArPtrProtocol, release), (unsigned)offsetof(ArPtrProtocol, release_ctx),
        AR_PTR_CONST, AR_PTR_NUL_TERMINATED, AR_PTR_STATIC);
    return 0;
}
'@
Set-Content -Path (Join-Path $work 'check.c') -Value $c -Encoding ascii
$env:PATH = "C:\Program Files (x86)\Microsoft Visual Studio\Installer;$env:PATH"
$bat = @"
@echo off
call "$vcvarsall" x64 >nul 2>nul
cd /d "$work"
cl /nologo /W4 /WX /utf-8 /I "$proto" check.c /Fe:check_c.exe >nul
if errorlevel 1 exit /b 1
cl /nologo /W4 /WX /utf-8 /TP /I "$proto" check.c /Fe:check_cpp.exe >nul
if errorlevel 1 exit /b 1
"@
Set-Content -Path (Join-Path $work 'build.bat') -Value $bat -Encoding ascii
Invoke-Limited 'cmd.exe' @('/c', (Join-Path $work 'build.bat')) 'msvc' | Out-Null
$cOut = Invoke-Limited (Join-Path $work 'check_c.exe') @() 'run_c'
$cppOut = Invoke-Limited (Join-Path $work 'check_cpp.exe') @() 'run_cpp'

# ── C# ───────────────────────────────────────────────────────────────────────
$csDir = Join-Path $work 'cs'
New-Item -ItemType Directory -Force $csDir | Out-Null
Copy-Item (Join-Path $proto 'ArPtrProtocol.cs') $csDir
@'
<Project Sdk="Microsoft.NET.Sdk">
  <PropertyGroup>
    <OutputType>Exe</OutputType>
    <TargetFramework>net8.0</TargetFramework>
    <AllowUnsafeBlocks>true</AllowUnsafeBlocks>
    <Nullable>enable</Nullable>
    <TreatWarningsAsErrors>true</TreatWarningsAsErrors>
  </PropertyGroup>
</Project>
'@ | Set-Content -Path (Join-Path $csDir 'check.csproj') -Encoding utf8
@'
using System;
using System.Runtime.InteropServices;
using ArrowInterop;
static string Off(string f) => Marshal.OffsetOf<ArPtrProtocol>(f).ToString();
Console.Write($"size={Marshal.SizeOf<ArPtrProtocol>()} address={Off("Address")} elem_type={Off("ElemType")} flags={Off("Flags")} _reserved={Off("Reserved")} length={Off("Length")} release={Off("Release")} release_ctx={Off("ReleaseCtx")} CONST={ArPtrProtocol.Const} NUL={ArPtrProtocol.NulTerminated} STATIC={ArPtrProtocol.Static}");
'@ | Set-Content -Path (Join-Path $csDir 'Program.cs') -Encoding utf8
$csOut = Invoke-Limited 'dotnet' @('run', '--project', (Join-Path $csDir 'check.csproj'), '-c', 'Release') 'dotnet'

# ── 比べる ──────────────────────────────────────────────────────────────────
$ok = $true
foreach ($pair in @(@('C', $cOut), @('C++', $cppOut), @('C#', $csOut))) {
    $got = $pair[1].Trim()
    if ($got -eq $expected) {
        Write-Host ("{0,-4} OK  {1}" -f $pair[0], $got)
    } else {
        Write-Host ("{0,-4} DIFF {1}" -f $pair[0], $got) -ForegroundColor Red
        $ok = $false
    }
}
Write-Host "Rust (expected) $expected"
if (-not $ok) { Write-Host 'PTR-PROTOCOL: MISMATCH' -ForegroundColor Red; exit 1 }
Write-Host 'PTR-PROTOCOL: consistent'
