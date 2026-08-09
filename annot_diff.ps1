# annot_diff.ps1 — #16 段階(c) の実測: ネイティブ codegen の自前型導出と
# AST 型解決層の注釈（AstAnnotations）の一致状況をモジュールごとに出力する。
#
# `AR_ANNOT_DIFF=1` を立てて `--compile` するだけ。生成物（.arc/.ars）は退避・復元する。

param(
    [switch]$Build
)

$ErrorActionPreference = 'Continue'
$repo = $PSScriptRoot

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
    Pop-Location
}

$exe = Join-Path $repo 'target\release\arrow.exe'
if (-not (Test-Path $exe)) { throw "not built: $exe" }

$env:AR_ANNOT_DIFF = '1'
try {
    foreach ($t in $targets) {
        $full = Join-Path $repo $t
        if (-not (Test-Path $full)) { Write-Host "skip (missing): $t"; continue }

        # 生成物を退避
        $bk = @()
        foreach ($e in @('.arc', '.ars')) {
            $a = [System.IO.Path]::ChangeExtension($full, $e)
            if (Test-Path $a) {
                $b = Join-Path $env:TEMP ('annotdiff_' + (($t -replace '[\\/]', '_') + $e))
                Copy-Item $a $b -Force
                $bk += , @($a, $b)
            }
        }

        Write-Host "=== $t" -ForegroundColor Cyan
        Push-Location $repo
        & $exe --compile $t | Out-Null
        Pop-Location

        foreach ($p in $bk) { Copy-Item $p[1] $p[0] -Force; Remove-Item $p[1] -Force }
    }
}
finally {
    Remove-Item Env:\AR_ANNOT_DIFF -ErrorAction SilentlyContinue
}
