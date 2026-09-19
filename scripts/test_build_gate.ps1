# test_build_gate.ps1 — `cargo test`（**debug**）がビルドできるかだけを見るゲート。
#
# ⚠⚠ **なぜ要るのか**: `src/vm/compiler/mod.rs` の `storage_operands`（op の網羅 `match`）は
#   `#[cfg(debug_assertions)]` なので **release ビルドには存在しない**。
#   `scripts/*.ps1` のゲートは全部 `target/release` を見るので、
#   **`Op` を足し忘れても全ゲートが緑のまま `cargo test` だけがビルド不能**になる。
#   同じ踏み方が 2 回起きている:
#     - 2026-09-08 `15aa677`  `Op::CoerceFloat`                         … **73 コミット**気づかず
#     - 2026-09-19 `c06254f` / `8c61d07`  `SeqExtend`/`SeqFinish`/`DictMerge` … **8 コミット**気づかず
#
# ⚠⚠ **`cargo test --release` は網にならない。** 強制点そのものがコンパイルされないので、
#   `--release` 付きだと「784 passed」と出たまま通ってしまう（2 回目はこれで見逃した）。
#   ⇒ **このスクリプトは `--release` を付けない。**
#
# ⚠ AST（`Stmt` / `Expr`）を変えたときの `src/frontend_tests/` の取りこぼしも、
#   テストが**別ビルド**である以上 `cargo build` では出ない。ここで出る。
#
# ⚠ 見るのは**ビルドが通るか**だけ。テストの合否は `cargo test` を別に走らせること
#   （このゲートは「毎回」走る側なので、実行時間を 0 に近く保つ）。
#   キャッシュが効いていれば 1 秒未満、`src/` を触った直後でも 20 秒程度。
#
# ⚠ `crates/arrow-frontend` は**ワークスペース外**（`Cargo.toml` の `exclude`）なので
#   ここには含まれない。あちらは `compare_wasm_frontend.ps1` が受け持つ。
#
# 使い方: ./scripts/test_build_gate.ps1
#   他のゲートからは `scan_examples.ps1` / `force_gate.ps1` が前段として呼ぶ
#   （そちらは `-SkipTestBuild` で飛ばせる）。

$ErrorActionPreference = 'Continue'
$repo = Split-Path -Parent $PSScriptRoot   # scripts/ の 1 つ上 = リポジトリ直下

Push-Location $repo
try {
    # ⚠ 出力はそのまま流す。PS 5.1 で native exe の stderr を `2>&1` で畳むと
    #   NativeCommandError になって成功でも $? が false になる（CLAUDE.md の注意）。
    cargo test --no-run --quiet
    $code = $LASTEXITCODE
}
finally {
    Pop-Location
}

if ($code -ne 0) {
    Write-Host ''
    Write-Host 'TEST-BUILD-GATE: FAILED — cargo test (debug) がビルドできない' -ForegroundColor Red
    Write-Host '  ⚠ release ゲートは全部緑のままになるので、ここで止めること。' -ForegroundColor Yellow
    Write-Host '  よくある原因: 新しい Op を vm/compiler/mod.rs の storage_operands に' -ForegroundColor Yellow
    Write-Host '                登録し忘れ / AST 変更に src/frontend_tests/ が追随していない。' -ForegroundColor Yellow
    exit 1
}

Write-Host 'TEST-BUILD-GATE: ok (cargo test --no-run, debug)' -ForegroundColor Green
exit 0
