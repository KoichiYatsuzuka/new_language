# dump_native_ir.ps1 — #16 段階(c) の検証用: ネイティブ codegen が生成する LLVM IR をダンプする。
#
# 対象モジュールを `--compile` して、生成 LLVM IR を <OutDir>\<name>.ll へ保存する。
# codegen 変更の前後で実行し `diff` を取ることで「IR が byte-identical か」を確認する。
#
# .arc / .ars は再生成されるとバイト列が変わる（埋め込み DLL）ため、
# 実行前に退避し実行後に復元する（作業ツリーを汚さない）。
#
# 使い方:
#   .\dump_native_ir.ps1 -OutDir <dir>          # release ビルド済みバイナリでダンプ
#   .\dump_native_ir.ps1 -OutDir <dir> -Build   # 先に cargo build --release も行う

param(
    [Parameter(Mandatory = $true)][string]$OutDir,
    [switch]$Build
)

$ErrorActionPreference = 'Stop'
$repo = $PSScriptRoot

# ネイティブ compile 対象の代表モジュール（codegen-eligible な関数を含むもの）
$targets = @(
    'examples/interop/test_modules/physics.ar',
    'examples/interop/test_modules/swd_nested.ar',
    'examples/interop/test_modules/typed_abi_module.ar',
    'examples/interop/geometry/__init__.ar',
    'examples/bench/flat_bench_module.ar',
    'examples/bench/partial_call_overhead_module.ar'
)

if ($Build) {
    Push-Location $repo
    cargo build --release
    if ($LASTEXITCODE -ne 0) { Pop-Location; throw 'cargo build failed' }
    Pop-Location
}

$exe = Join-Path $repo 'target\release\arrow.exe'
if (-not (Test-Path $exe)) { throw "not built: $exe" }

New-Item -ItemType Directory -Force $OutDir | Out-Null

# 生成物（.arc/.ars）を退避
$backupDir = Join-Path $OutDir '_artifact_backup'
New-Item -ItemType Directory -Force $backupDir | Out-Null
$saved = @()
foreach ($t in $targets) {
    $full = Join-Path $repo $t
    foreach ($ext in @('.arc', '.ars')) {
        $artifact = [System.IO.Path]::ChangeExtension($full, $ext)
        if (Test-Path $artifact) {
            $key = ($t -replace '[\\/]', '_') + $ext
            Copy-Item $artifact (Join-Path $backupDir $key) -Force
            $saved += , @($artifact, (Join-Path $backupDir $key))
        }
    }
}

try {
    foreach ($t in $targets) {
        $full = Join-Path $repo $t
        if (-not (Test-Path $full)) { Write-Host "skip (missing): $t"; continue }

        $name = [System.IO.Path]::GetFileNameWithoutExtension($full)
        if ($name -eq '__init__') {
            $name = (Split-Path (Split-Path $full -Parent) -Leaf) + '__init__'
        }
        $ll = Join-Path $OutDir "$name.ll"
        if (Test-Path $ll) { Remove-Item $ll -Force }

        $env:AR_DUMP_LL = $ll
        Push-Location $repo
        # native exe の stderr は PS5.1 で ErrorRecord 化されるため、ここだけ非停止にする
        $prev = $ErrorActionPreference
        $ErrorActionPreference = 'Continue'
        & $exe --compile $t | Out-Null
        $ErrorActionPreference = $prev
        Pop-Location
        Remove-Item Env:\AR_DUMP_LL

        if (Test-Path $ll) {
            Write-Host ("dumped {0,-28} {1} bytes" -f $name, (Get-Item $ll).Length)
        } else {
            Write-Host "no IR (no eligible fns): $t"
        }
    }
}
finally {
    # 退避した生成物を書き戻す
    foreach ($p in $saved) { Copy-Item $p[1] $p[0] -Force }
    Remove-Item $backupDir -Recurse -Force
}
