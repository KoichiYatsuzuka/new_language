# compare_outputs.ps1 — 2 つの arrow.exe で **`gate/` の全テストの exit / stdout / stderr / 機械向けの診断が同一か**を見る。
#
# ⚠⚠ **[compare_bytecode.ps1](scripts/compare_bytecode.ps1) では足りない場合のためのゲート**（#63 で新設）。
#    バイトコード同一性が言えるのは「コンパイラを触っていない」ことだけで、
#    **解釈側（VM の実行・組み込み関数・外部言語の橋渡し）だけを触った変更では自明に一致してしまう**。
#    ⇒ **解釈側を触ったら bytecode ではなくこちらで「挙動不変」を主張すること。**
#
# 使い方:
#   ./scripts/compare_outputs.ps1 -A <base-arrow.exe> [-B <new-arrow.exe>]   # -B の既定は target/release/arrow.exe
#   ./scripts/compare_outputs.ps1 -A x.exe -B x.exe       # 負の対照（独立した 2 回の実行を比べる。必ず 100% 一致）
#   ./scripts/compare_outputs.ps1 -A x.exe -ShowDiff      # 差分の中身も表示
#   -Fresh で保存を使わず両方走らせ直す。-Root で別の gate/（worktree など）を指す。
#
# ── 2026-11-23 からの形（gate_speed_plan.md 6-1 / 6-2）──────────────────────────
#   - 対象は `examples/` ではなく **`gate/`**（gates_every_time と同じ 762 件前後・async とベンチも含む。gate/ のテストは
#     同じ exe で 2 回走らせて揃う書き方にしてある・gate/README.md 7 節）。GUI・対話は gate/ に無い（D-10）。
#   - 実行は gate_ab.ps1 の `Invoke-GateAbSides`: compare_bytecode と**同じ 1 回の実行**（`AR_VM_DUMP=1`）を読み、ここでは
#     stderr からダンプを除いて比べる。B の結果は「毎回」のゲートが残したもの、A の結果は exe のハッシュで保存したものを
#     読む（読んだことは `gate_run: reused …` と出る）。
#   - cpp-lib の中継 DLL のキャッシュは exe ごとに温めて分け、終わったら始める前に戻す（gate_ab.ps1・7-34）。
#
# ⚠⚠ **使う前に必ず同一 exe 同士で負の対照を取る**（-A と -B に同じ exe。gate_ab.ps1 が別の保存先に走らせ直す）。
#    #63 の初回は 13 例題が DIFFERS になり、原因は ①経過時間 ②オブジェクトアドレス（`0x…` / `id()`）の 2 種だった。
#    前者は gate/ のベンチが偽の時計を使うので出ない。後者は `ConvertTo-GateAbStable` で正規化してある。
# ⚠ A/B の基準（-A）は「直前のタスクのコミット」からビルドした exe にすること（CLAUDE.md）。
# ⚠ このファイルは**日本語コメントを含むので UTF-8 BOM 付きで保存すること**。
param(
    [Parameter(Mandatory=$true)][string]$A,
    [string]$B = '',
    [string]$Root = '',
    [int]$Jobs = 0,
    [switch]$Fresh,
    [switch]$ShowDiff
)

$ErrorActionPreference = 'Continue'
$repo = Split-Path -Parent $PSScriptRoot   # scripts/ の 1 つ上 = リポジトリ直下
if ([string]::IsNullOrEmpty($B)) { $B = Join-Path $repo 'target/release/arrow.exe' }
if ([string]::IsNullOrEmpty($Root)) { $Root = Join-Path $repo 'gate' }
. (Join-Path $PSScriptRoot 'gate_ab.ps1')

# ⚠⚠ **相対パスを絶対パスへ正規化する**（タスク 9.5）。`Process.Start` の `FileName` は `WorkingDirectory` ではなく
#    呼び出し側プロセスのカレントで解決するので、相対パスだと基準バイナリが見つからず全件が差分という嘘の結果が出た。
$A = Resolve-GateAbExe $A
$B = Resolve-GateAbExe $B

$sides = Invoke-GateAbSides -A $A -B $B -Root $Root -Jobs $Jobs -Fresh:$Fresh
exit (Compare-GateAbResults $sides ${function:Get-GateAbOutputRecord} 'outputs' -ShowDiff:$ShowDiff)
