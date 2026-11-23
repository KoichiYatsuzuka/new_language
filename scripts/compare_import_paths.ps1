# compare_import_paths.ps1 — 2 つの arrow.exe で **import 系のテストの exit / stdout / stderr / 機械向けの診断が同一か**を見る。
#
# #58（`Stmt::Import` アーム 210 行の切り出し）で cs-dll / cs-proc / js-proc の
# **早期 return を共通の末尾へ畳んだ**ので、「束縛名が同じになる」ことを実測で裏づけるための検査。
# import / FFI を触ったときに、全件の compare_outputs.ps1 より先に短く回す用（13 件・数秒）。
#
# 使い方:
#   ./scripts/compare_import_paths.ps1 -A <base-arrow.exe> [-B <new-arrow.exe>]   # -B の既定は target/release/arrow.exe
#   ./scripts/compare_import_paths.ps1 -A x.exe -B x.exe      # 負の対照（独立した 2 回の実行を比べる。必ず 100% 一致）
#   -ShowDiff で差分の中身、-Fresh で保存を使わず走らせ直す。
#
# ── 2026-11-23 からの形（gate_speed_plan.md 6-3）──────────────────────────────
#   - 対象は `examples/interop/` ではなく、その写しの `gate/ffi/`（4-4〜4-7）。実行と保存は compare_outputs と同じ仕組み
#     （gate_ab.ps1）で、比べ方も同じ（ダンプを除いた stderr と診断も比べる）。基準の exe の結果は exe のハッシュで保存する。
#   - ⚠ 13 件は compare_outputs の対象（gate/ の全件）にも含まれる。ここは一覧が違うので別に 1 回走らせる（保存のキーが違う）。
#   - ⚠ 子の出力はパイプで受けない（#58 で実際にハングさせた。js-proc のブリッジの孫プロセスがパイプを握る）。共通ランナーは
#     ファイルへ落とす。
#   - cpp-lib の中継 DLL のキャッシュは exe ごとに温めて分け、終わったら始める前に戻す（gate_ab.ps1・7-34）。
# ⚠ このファイルは**日本語コメントを含むので UTF-8 BOM 付きで保存すること**。
param(
    [Parameter(Mandatory=$true)][string]$A,
    [string]$B = '',
    [int]$Jobs = 0,
    [switch]$Fresh,
    [switch]$ShowDiff
)

$ErrorActionPreference = 'Continue'
$repo = Split-Path -Parent $PSScriptRoot   # scripts/ の 1 つ上 = リポジトリ直下
if ([string]::IsNullOrEmpty($B)) { $B = Join-Path $repo 'target/release/arrow.exe' }
. (Join-Path $PSScriptRoot 'gate_ab.ps1')

# ⚠⚠ **相対パスを絶対パスへ正規化する**（タスク 9.5・compare_outputs.ps1 と同じ）。
$A = Resolve-GateAbExe $A
$B = Resolve-GateAbExe $B

# import[cs-dll] / [cs-proc] / [js-proc] / [cpp-dll] / [cpp-lib] と Python の探索パスを踏むテスト（gate/ からの相対）。
$targets = @(
    'ffi/cs_interop_test.ar',
    # ⚠ `ar_config.json` の `python.search_paths` を踏むテスト群（#61/#69/#74）。
    #    `py_subdir_tests/` のものはサブディレクトリから**祖先の** `ar_config.json` を読む（4-7）。
    'ffi/import_py_search_path.ar',
    'ffi/import_py_int_search_path.ar',
    'ffi/py_subdir_tests/import_py_ancestor_config.ar',
    # ⚠ cs-proc: 以前は窓を出す `cs_proc_app` が唯一の例題だった。gate/ には窓を出さない cs-proc のテストがあるので、
    #    値の受け渡しを広く踏む `cs_proc_values` を代わりに置く。
    'ffi/cs_proc_values.ar',
    'ffi/event_cs_fire.ar',
    'ffi/event_cs_handler.ar',
    'ffi/js_proc_test.ar',
    'ffi/js_proc_async_test.ar',
    'ffi/cpp_struct_ptr.ar',
    'ffi/cpp_struct_ptr_error.ar',
    'ffi/cpp_out_param_writeback.ar',
    'ffi/cpp_default_arg_native_call.ar'
)
$gate = Join-Path $repo 'gate'
$missing = @($targets | Where-Object { -not (Test-Path (Join-Path $gate $_)) })
if ($missing.Count -gt 0) {
    # ⚠ 黙って減らさない（名前を変えたら一覧も直す）
    $missing | ForEach-Object { Write-Host ("{0,-60} MISSING" -f $_) -ForegroundColor Red }
    exit 1
}

$sides = Invoke-GateAbSides -A $A -B $B -Root $gate -Files @($targets | ForEach-Object { Join-Path $gate $_ }) -Jobs $Jobs -Fresh:$Fresh
exit (Compare-GateAbResults $sides ${function:Get-GateAbOutputRecord} 'import paths' -ShowDiff:$ShowDiff -DiffLines 30)
