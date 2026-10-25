# check_native_ir_abi.ps1 — ネイティブ codegen の内部 ABI の検査（native_object_access_plan.md 4-1）
#
# 例題の .arc の元（.ar）を全部いまの exe でコンパイルし直し（作業ツリーは触らない・%TEMP% の写しで行う）、
# 生成 LLVM IR の**直接呼び出し（@X_impl / @X_fast）が定義と一致するか**（戻り値の型・引数の型）を見る。
#
# ⚠⚠ なぜ要るのか: LLVM は opaque pointer 越しの型の違う直接呼び出しを通すので、codegen が `call i64` で
#   `double` を返す関数を呼んでも clang は落ちず、**実行時にごみを読む**（3-2 で `float * None` になった）。
#   例題の出力は .arc を作り直さないと変わらず、作り直しても値によっては表に出ない。
# ⚠ あわせて、**DLL が埋め込まれなかったモジュール**も失敗にする（1-8: オーバーロードで clang が落ち、
#   `--compile` が「skipped」と出して DLL の無い .arc を黙って書いていた）。
#
# 使い方:
#   .\scripts\check_native_ir_abi.ps1                 # target\release\arrow.exe で検査
#   .\scripts\check_native_ir_abi.ps1 -Exe <exe>
# 最後の行: NATIVE-IR-ABI: ok / NATIVE-IR-ABI: FAILED（exit 1）

param(
    [string]$Exe = '',
    [int]$LimitSec = 300
)

$ErrorActionPreference = 'Continue'
$repo = Split-Path -Parent $PSScriptRoot
if (-not $Exe) { $Exe = Join-Path $repo 'target\release\arrow.exe' }
$work = Join-Path $env:TEMP "ar_native_ir_abi_$PID"
New-Item -ItemType Directory -Force $work | Out-Null
$llDir = Join-Path $work '_ll'
New-Item -ItemType Directory -Force $llDir | Out-Null

# .arc を持つ例題だけを写す。stale_arc は「古い .arc を読まない」例題、archived / practical_examples は
# 例題のゲートの対象外（CLAUDE.md・後者は DxLib など外部の資源が要る）なので除く。
$sources = Get-ChildItem -Recurse (Join-Path $repo 'examples') -Filter *.arc |
    Where-Object { $_.FullName -notmatch '\\archived\\|\\practical_examples\\|\\stale_arc\\' } |
    ForEach-Object { [IO.Path]::ChangeExtension($_.FullName, '.ar') } |
    Where-Object { Test-Path $_ }

$failed = 0
$n = 0
foreach ($src in $sources) {
    $n++
    $rel = $src.Substring($repo.Length + 1)
    $dst = Join-Path $work $rel
    New-Item -ItemType Directory -Force (Split-Path $dst) | Out-Null
    Copy-Item $src $dst
    $ll = Join-Path $llDir (($rel -replace '[\\/:]', '_') -replace '\.ar$', '.ll')
    $psi = New-Object System.Diagnostics.ProcessStartInfo
    $psi.FileName = $Exe
    $psi.Arguments = '--compile "' + $dst + '"'
    $psi.WorkingDirectory = $repo   # ar_config.json（clang の場所）を引くため
    $psi.UseShellExecute = $false; $psi.RedirectStandardOutput = $true; $psi.RedirectStandardError = $true
    $psi.EnvironmentVariables['AR_DUMP_LL'] = $ll
    $p = [Diagnostics.Process]::Start($psi)
    $o = $p.StandardOutput.ReadToEndAsync(); $e = $p.StandardError.ReadToEndAsync()
    if (-not $p.WaitForExit($LimitSec * 1000)) {
        & taskkill /T /F /PID $p.Id | Out-Null
        Write-Host "TIMEOUT  $rel"
        $failed++
        continue
    }
    $p.WaitForExit()
    $out = $o.Result + $e.Result
    if ($out -notmatch 'embedded in') {
        $why = ($out -split "`n" | Where-Object { $_ -match 'error|skipped' } | Select-Object -First 1)
        Write-Host "NO-DLL   $rel  $why"
        $failed++
    }
}

$lls = @(Get-ChildItem $llDir -Filter *.ll | ForEach-Object { $_.FullName })
$check = & python (Join-Path $PSScriptRoot 'native_ir_abi_check.py') $lls 2>&1
$check | Where-Object { $_ -notmatch 'internal function\(s\)' } | ForEach-Object { Write-Host $_ }
if ($LASTEXITCODE -ne 0) { $failed++ }

Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue
Write-Host "modules: $n   IR files: $($lls.Count)"
if ($failed -eq 0) { Write-Host 'NATIVE-IR-ABI: ok' } else { Write-Host 'NATIVE-IR-ABI: FAILED'; exit 1 }
