# bench.ps1 — ツリーウォークインタープリタのベースライン計測（バイトコードVM移行フェーズ0）
#
# Usage: ./bench.ps1            # 全ベンチ実行
#        ./bench.ps1 -Reps 3    # 各ベンチを3回実行して安定性を見る
#
# BYTECODE_VM_PLAN.md フェーズ0の判断ゲート用。名前引き支配 vs Value クローン支配を切り分ける。

param(
    [int]$Reps = 3
)

$ErrorActionPreference = "Stop"
$exe = "target/release/arrow.exe"

if (-not (Test-Path $exe)) {
    Write-Host "release binary not found. Running: cargo build --release" -ForegroundColor Yellow
    cargo build --release
}

function Run-Bench($label, $script) {
    Write-Host ""
    Write-Host "==== $label ====" -ForegroundColor Cyan
    Write-Host "($script)" -ForegroundColor DarkGray
    for ($r = 1; $r -le $Reps; $r++) {
        Write-Host "-- run $r/$Reps --" -ForegroundColor DarkGray
        & $exe -src $script
    }
}

Run-Bench "bottleneck (要因分離)" "examples/bench/bottleneck_bench.ar"
Run-Bench "field access (E2E)"    "examples/bench/bench_field_access.ar"
Run-Bench "string (#15 A/B)"      "examples/bench/bench_string.ar"
Run-Bench "name hash probe (#15-3)" "examples/bench/bench_name_hash.ar"

Write-Host ""
Write-Host "done." -ForegroundColor Green
