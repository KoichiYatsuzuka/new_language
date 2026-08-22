# compare_import_paths.ps1 — 2 つの arrow.exe で **import 系の例題の stdout/stderr/exit が同一か**を見る。
#
# #58（`Stmt::Import` アーム 210 行の切り出し）で cs-dll / cs-proc / js-proc の
# **早期 return を共通の末尾へ畳んだ**ので、「束縛名が同じになる」ことを実測で裏づけるための検査。
# これらの例題は [compare_bytecode.ps1](compare_bytecode.ps1) の対象外（GUI・外部プロセス）なので、
# **バイトコード同一性だけでは #58 の主張を支えられない**。
#
# 使い方:
#   ./compare_import_paths.ps1 -A <head-arrow.exe> -B <new-arrow.exe>
#   ./compare_import_paths.ps1 -A x.exe -B x.exe      # 負の対照（必ず 100% 一致）
#
# ⚠⚠ **子の出力をパイプで受けてはいけない**（#58 で実際にハングさせた）。
#    `import[js-proc]` は **node のブリッジを孫プロセスとして起こし、それが生き残る**。
#    孫はパイプの書き込み端を握ったままなので、`arrow.exe` が終了して `WaitForExit` が返っても
#    `ReadToEndAsync` が**永久に完了しない**。⇒ ここでは `Start-Process -RedirectStandard*` で
#    **ファイルへ落としてから読む**（ファイルハンドルは孫が持っていても読み出しを妨げない）。
#    ⚠ [compare_bytecode.ps1](compare_bytecode.ps1) が js_proc 系を skip しているのは同じ理由。
#
# ⚠ GUI 例題（cs_form_app / DxLib 系）は窓が出て終わらないので**対象外**。
#    ここで見るのは「import が名前空間を作って束縛するところまで」で足りる。
# ⚠ このファイルは**日本語コメントを含むので UTF-8 BOM 付きで保存すること**。
param(
    [Parameter(Mandatory=$true)][string]$A,
    [string]$B = '',
    [int]$TimeoutSec = 60,
    [switch]$ShowDiff
)

$ErrorActionPreference = 'Continue'
$repo = $PSScriptRoot
if ([string]::IsNullOrEmpty($B)) { $B = Join-Path $repo 'target/release/arrow.exe' }

foreach ($exe in @($A, $B)) {
    if (-not (Test-Path $exe)) { Write-Host "NOT FOUND: $exe" -ForegroundColor Red; exit 2 }
}

# import[cs-dll] / [cs-proc] / [js-proc] / [cpp-dll] / [cpp-lib] を踏む非 GUI 例題。
$targets = @(
    'examples/interop/cs_interop_test.ar',
    # ⚠ **`import[cs-proc]` を踏む唯一の非 GUI 例題**（#58 で気づいた）。
    #    [compare_bytecode.ps1](compare_bytecode.ps1) は外部プロセスを理由に skip しているので、
    #    これを外すと cs-proc 経路を見る網が 1 つも無くなる。
    'examples/interop/cs_proc_app.ar',
    'examples/interop/event_cs_fire.ar',
    'examples/interop/event_cs_handler.ar',
    'examples/interop/js_proc_test.ar',
    'examples/interop/js_proc_async_test.ar',
    'examples/interop/cpp_struct_ptr.ar',
    'examples/interop/cpp_struct_ptr_error.ar',
    'examples/interop/cpp_out_param_writeback.ar',
    'examples/interop/cpp_default_arg_native_call.ar'
)

$tmp = Join-Path ([System.IO.Path]::GetTempPath()) ("ar_imp_" + [guid]::NewGuid().ToString('N'))
$null = New-Item -ItemType Directory -Path $tmp

function Get-Output([string]$exe, [string]$file, [string]$tag) {
    $o = Join-Path $tmp "$tag.out"
    $e = Join-Path $tmp "$tag.err"
    $p = Start-Process -FilePath $exe -ArgumentList ('"' + $file + '"') `
        -WorkingDirectory $repo -NoNewWindow -PassThru `
        -RedirectStandardOutput $o -RedirectStandardError $e
    if (-not $p.WaitForExit($TimeoutSec * 1000)) {
        try { $p.Kill() } catch {}
        return $null
    }
    $so = ''; $se = ''
    if (Test-Path $o) { $so = (Get-Content $o -Raw -Encoding UTF8) }
    if (Test-Path $e) { $se = (Get-Content $e -Raw -Encoding UTF8) }
    if ($null -eq $so) { $so = '' }
    if ($null -eq $se) { $se = '' }
    $so = $so -replace "`r`n", "`n"
    $se = $se -replace "`r`n", "`n"
    return "EXIT=$($p.ExitCode)`n--- STDOUT ---`n$so`n--- STDERR ---`n$se"
}

$same = 0; $diff = 0; $missing = 0; $timeouts = 0
$rows = @()
$i = 0

foreach ($rel in $targets) {
    $i++
    $full = Join-Path $repo $rel
    if (-not (Test-Path $full)) {
        $missing++
        $rows += ("{0,-48} MISSING" -f $rel)
        continue
    }
    $oa = Get-Output $A $full "a$i"
    $ob = Get-Output $B $full "b$i"
    if (($null -eq $oa) -or ($null -eq $ob)) {
        $timeouts++
        $rows += ("{0,-48} TIMEOUT (not compared)" -f $rel)
        continue
    }
    if ($oa -eq $ob) {
        $same++
    } else {
        $diff++
        $rows += ("{0,-48} DIFFERS" -f $rel)
        if ($ShowDiff) {
            $rows += (Compare-Object ($oa -split "`n") ($ob -split "`n") |
                Select-Object -First 30 |
                ForEach-Object { "    {0} {1}" -f $_.SideIndicator, $_.InputObject })
        }
    }
}

try { Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue } catch {}

if ($rows.Count -gt 0) {
    Write-Host 'DIFFERING / NOT COMPARED:' -ForegroundColor Yellow
    $rows | ForEach-Object { Write-Host $_ }
    Write-Host ''
}
Write-Host ("import paths identical: {0} / {1}   differing: {2}   missing: {3}   timeout: {4}" -f `
    $same, ($same + $diff), $diff, $missing, $timeouts)
if ($diff -gt 0) { exit 1 } else { exit 0 }
