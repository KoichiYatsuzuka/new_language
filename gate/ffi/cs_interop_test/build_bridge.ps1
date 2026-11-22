# build_bridge.ps1 — ArrowBridge の export 生成 + NativeAOT 再発行 + .ars スタブ再生成
#
# 1. dotnet build（マネージド・生成した export を除く）→ そのアセンブリから export を生成する
#    （arrow --gen-cs-exports → ArrowExports.g.cs・ffi_value_exchange_plan.md 5-1。以前は ArrowExports.cs を手書きしていた）
# 2. dotnet publish (NativeAOT・生成した export を含む) → ArrowBridge_native.dll に配置
# 3. マネージド DLL → ArrowBridge.dll に配置 (ECMA-335 メタデータ読み取り用)
# 4. arrow --compile-cs で ArrowBridge.ars を再生成
#
# ⚠ 1 回目のビルドは ArrowExports.g.cs を外す（-p:ArrowGenerated=false・ArrowBridge.csproj）。古い生成物が
#   消えたメソッドを呼んでいてもビルドが通るように。生成したクラスは internal なので、型スタブには出ない。

$ErrorActionPreference = "Stop"
$dir = Split-Path $MyInvocation.MyCommand.Path
$repoRoot = (Resolve-Path "$dir\..\..\..").Path

# vswhere.exe を PATH に追加 (NativeAOT ビルドに必要)
$vswhere = "C:\Program Files (x86)\Microsoft Visual Studio\Installer"
$env:PATH = "$vswhere;$env:PATH"

# cargo writes progress ("Compiling ...") to stderr. Under $ErrorActionPreference = "Stop", PowerShell 5.1 turns
# that into a terminating NativeCommandError, so the step failed whenever cargo had to compile. Judge by the exit code.
function Invoke-Arrow([string[]]$arrowArgs) {
    Push-Location $repoRoot
    try {
        $ErrorActionPreference = "Continue"
        cargo run --release -- @arrowArgs 2>&1 | ForEach-Object { "$_" }
        if ($LASTEXITCODE -ne 0) { Write-Error "arrow $($arrowArgs -join ' ') failed ($LASTEXITCODE)"; exit 1 }
    } finally {
        Pop-Location
    }
}

Push-Location $dir
try {
    Write-Host "=== Restoring packages ===" -ForegroundColor Cyan
    dotnet restore ArrowBridge.csproj -r win-x64

    Write-Host "=== Building the managed DLL (without the generated exports) ===" -ForegroundColor Cyan
    dotnet build ArrowBridge.csproj -r win-x64 -c Release -p:ArrowGenerated=false
    if ($LASTEXITCODE -ne 0) { Write-Error "dotnet build failed ($LASTEXITCODE)"; exit 1 }
    $managed = "$dir\bin\Release\net8.0\win-x64\ArrowBridge.dll"
    if (-not (Test-Path $managed)) { Write-Warning "Managed DLL not found at: $managed"; exit 1 }
} finally {
    Pop-Location
}

Write-Host "=== Generating ArrowExports.g.cs ===" -ForegroundColor Cyan
Invoke-Arrow @("--gen-cs-exports", $managed, "-o", "$dir\ArrowExports.g.cs")

Push-Location $dir
try {
    Write-Host "=== Publishing NativeAOT DLL ===" -ForegroundColor Cyan
    dotnet publish ArrowBridge.csproj `
        -r win-x64 `
        -c Release `
        --self-contained
    if ($LASTEXITCODE -ne 0) { Write-Error "dotnet publish failed ($LASTEXITCODE)"; exit 1 }

    $native = "$dir\bin\Release\net8.0\win-x64\publish\ArrowBridge.dll"
    $dest   = "$dir\ArrowBridge_native.dll"
    if (Test-Path $native) {
        Copy-Item -Force $native $dest
        Write-Host "=== Copied NativeAOT DLL to $dest ===" -ForegroundColor Green
    } else {
        Write-Warning "NativeAOT DLL not found at: $native"
        exit 1
    }

    # マネージド DLL (ECMA-335 メタデータ読み取り用・2 回目のビルドのもの)
    if (Test-Path $managed) {
        Copy-Item -Force $managed "$dir\ArrowBridge.dll"
        Write-Host "=== Copied managed DLL to $dir\ArrowBridge.dll ===" -ForegroundColor Green
    } else {
        Write-Warning "Managed DLL not found"
        exit 1
    }
} finally {
    Pop-Location
}

Write-Host "=== Regenerating ArrowBridge.ars stub ===" -ForegroundColor Cyan
Invoke-Arrow @("--compile-cs", "examples/interop/cs_interop_test/ArrowBridge.dll")
Write-Host "=== Build complete ===" -ForegroundColor Green
