# compare_bytecode.ps1 — 2 つの arrow.exe が生成する**バイトコードが同一か**を `gate/` の全テストで突き合わせる。
#
# 「挙動不変」の主張を exit code より強く裏付けるための検査（#52 で確立した手順の .ps1 化・#68）。
# ⚠ バイトコードが同一でも、解釈側だけを触った変更では挙動が変わりうる。そちらは compare_outputs.ps1 の担当。
#
# 使い方:
#   ./scripts/compare_bytecode.ps1 -A <base-arrow.exe> [-B <new-arrow.exe>]   # -B の既定は target/release/arrow.exe
#   ./scripts/compare_bytecode.ps1 -A x.exe -B x.exe      # 負の対照（独立した 2 回の実行を比べる。必ず 100% 一致）
#   -ShowDiff で差分の中身、-Fresh で保存を使わず走らせ直す、-Root で別の gate/ を指す。
#
# ── 2026-11-23 からの形（gate_speed_plan.md 6-1 / 6-2）──────────────────────────
#   - 対象は `examples/` ではなく **`gate/`**。実行は compare_outputs と**同じ 1 回**（gate_ab.ps1・`AR_VM_DUMP=1`）で、
#     ここでは stderr のダンプの行（`Split-GateVmDump`）だけを比べる。
#   - ⚠ stderr にはダンプ以外も混ざる（実行時の誤りのトレースバックなど）。このゲートの主張は「バイトコードが同一か」で
#     stderr の一致ではない（それは compare_outputs の担当）ので、ダンプの塊だけを取り出す。
#   - ⚠ 以前の形は子の出力をパイプで受けていたので、外部プロセス（js-proc / cs-proc）と長いベンチを外していた。共通ランナーは
#     ファイルへ落とすので、それらも比べる。
#
# ⚠⚠ 差分を見たら**まず同一 exe で再現するかを見る**（-A と -B に同じ exe を渡す）。async のテストは、worker スレッドの
#    コンパイルの順がスケジューリングで変わりうる（#52）。
# ⚠ このファイルは**日本語コメントを含むので UTF-8 BOM 付きで保存すること**（PS5.1 は BOM 無しを ANSI として読む）。
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

# ⚠⚠ **相対パスを絶対パスへ正規化する**（タスク 9.5・compare_outputs.ps1 と同じ）。
$A = Resolve-GateAbExe $A
$B = Resolve-GateAbExe $B

$sides = Invoke-GateAbSides -A $A -B $B -Root $Root -Jobs $Jobs -Fresh:$Fresh
exit (Compare-GateAbResults $sides ${function:Get-GateAbDumpRecord} 'bytecode' -ShowDiff:$ShowDiff)
