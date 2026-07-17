# build_bridge.ps1 — ArrowBridge の NativeAOT 再発行 + .ars スタブ再生成
#
# 1. dotnet publish (NativeAOT) → ArrowBridge_native.dll に配置
# 2. マネージド DLL → ArrowBridge.dll に配置 (ECMA-335 メタデータ読み取り用)
# 3. cargo run -- --compile-cs で ArrowBridge.ars を再生成

$ErrorActionPreference = "Stop"
$dir = Split-Path $MyInvocation.MyCommand.Path
$repoRoot = (Resolve-Path "$dir\..\..\..").Path

# vswhere.exe を PATH に追加 (NativeAOT ビルドに必要)
$vswhere = "C:\Program Files (x86)\Microsoft Visual Studio\Installer"
$env:PATH = "$vswhere;$env:PATH"

Push-Location $dir
try {
    Write-Host "=== Restoring packages ===" -ForegroundColor Cyan
    dotnet restore ArrowBridge.csproj

    Write-Host "=== Publishing NativeAOT DLL ===" -ForegroundColor Cyan
    dotnet publish ArrowBridge.csproj `
        -r win-x64 `
        -c Release `
        --self-contained

    $native = "$dir\bin\Release\net8.0\win-x64\publish\ArrowBridge.dll"
    $dest   = "$dir\ArrowBridge_native.dll"
    if (Test-Path $native) {
        Copy-Item -Force $native $dest
        Write-Host "=== Copied NativeAOT DLL → $dest ===" -ForegroundColor Green
    } else {
        Write-Warning "NativeAOT DLL not found at: $native"
        exit 1
    }

    # マネージド DLL (ECMA-335 メタデータ読み取り用)
    $managed = "$dir\bin\Release\net8.0\win-x64\ArrowBridge.dll"
    if (-not (Test-Path $managed)) {
        $managed = "$dir\bin\Release\net8.0\ArrowBridge.dll"
    }
    if (Test-Path $managed) {
        Copy-Item -Force $managed "$dir\ArrowBridge.dll"
        Write-Host "=== Copied managed DLL → $dir\ArrowBridge.dll ===" -ForegroundColor Green
    } else {
        Write-Warning "Managed DLL not found"
        exit 1
    }
} finally {
    Pop-Location
}

Write-Host "=== Regenerating ArrowBridge.ars stub ===" -ForegroundColor Cyan
Push-Location $repoRoot
try {
    cargo run -- --compile-cs examples/interop/cs_interop_test/ArrowBridge.dll
} finally {
    Pop-Location
}
Write-Host "=== Build complete ===" -ForegroundColor Green
