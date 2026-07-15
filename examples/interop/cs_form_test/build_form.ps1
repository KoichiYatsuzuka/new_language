$ErrorActionPreference = "Stop"
$dir = Split-Path $MyInvocation.MyCommand.Path

# vswhere.exe を PATH に追加 (NativeAOT ビルドに必要)
$vswhere = "C:\Program Files (x86)\Microsoft Visual Studio\Installer"
$env:PATH = "$vswhere;$env:PATH"

Push-Location $dir
try {
    Write-Host "=== Restoring packages ===" -ForegroundColor Cyan
    dotnet restore FormBridge.csproj

    Write-Host "=== Publishing NativeAOT DLL ===" -ForegroundColor Cyan
    dotnet publish FormBridge.csproj `
        -r win-x64 `
        -c Release `
        --self-contained

    $native = "$dir\bin\Release\net8.0-windows\win-x64\publish\FormBridge.dll"
    $dest   = "$dir\FormBridge_native.dll"
    if (Test-Path $native) {
        Copy-Item -Force $native $dest
        Write-Host "=== Copied NativeAOT DLL → $dest ===" -ForegroundColor Green
    } else {
        Write-Warning "NativeAOT DLL not found at: $native"
        exit 1
    }

    # マネージド DLL (ECMA-335 メタデータ読み取り用)
    $managed = "$dir\bin\Release\net8.0-windows\win-x64\FormBridge.dll"
    if (-not (Test-Path $managed)) {
        # フォールバック: 通常ビルド出力
        dotnet build FormBridge.csproj -c Release
        $managed = "$dir\bin\Release\net8.0-windows\FormBridge.dll"
    }
    if (Test-Path $managed) {
        Copy-Item -Force $managed "$dir\FormBridge.dll"
        Write-Host "=== Copied managed DLL → $dir\FormBridge.dll ===" -ForegroundColor Green
    }

    Write-Host "=== Build complete ===" -ForegroundColor Green
} finally {
    Pop-Location
}
