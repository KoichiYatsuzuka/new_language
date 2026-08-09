# compare_vm_modes.ps1 — `--vm=off` と `--vm=auto` の出力が byte-identical かを検証する。
#
# バイトコード VM / AST 型解決層（#16）の変更は「解釈経路の観測可能な挙動を変えない」ことが
# 不変条件なので、各変更後にこれを実行して回帰を確認する。
#
# 使い方:
#   .\compare_vm_modes.ps1              # 非エラー例題を全件比較
#   .\compare_vm_modes.ps1 -Filter phys # ファイル名部分一致で絞り込み

param(
    [string]$Filter = '',
    # examples/bench は経過時間を出力するため off/auto で一致しない（比較対象外が既定）
    [switch]$IncludeBench,
    # 1 例題あたりの上限秒数（超えたら kill して TIMEOUT 扱い）
    [int]$TimeoutSec = 60
)

$ErrorActionPreference = 'Continue'

$exe = Join-Path $PSScriptRoot 'target\release\arrow.exe'
if (-not (Test-Path $exe)) { throw "not built: $exe (run: cargo build --release)" }

# 対話入力・外部 DLL 依存・意図的スキップ（run_examples.ps1 と同じ方針）
$skip = @(
    'debug_demo', 'async_bench', 'async_demo', 'spider_render', 'spider_solitaire',
    'rs_struct', 'flat_bench', 'flat_bench_interp', 'flat_bench_module',
    # GUI ウィンドウ / 外部プロセス（Node・描画）を開いたままになるため比較不能
    'cs_form_app', 'cs_proc_app', 'js_proc_test', 'js_proc_async_test', 'math_render',
    # import[rs] sha2 のクレートが rust.crates_path に無い環境では実行できない
    'importation'
)

$categoryDirs = @('basics', 'collections', 'classes', 'typing', 'exceptions', 'async', 'apps', 'interop')
if ($IncludeBench) { $categoryDirs += 'bench' }

$examples = $categoryDirs | ForEach-Object { Get-ChildItem "$PSScriptRoot\examples\$_\*.ar" -ErrorAction SilentlyContinue } | Where-Object {
    $name = $_.BaseName
    (-not ($name -match '_error' -or $name -match '__errors' -or $skip -contains $name)) -and
    ($Filter -eq '' -or $name -like "*$Filter*")
} | Sort-Object Name

# 1 例題を指定 VM モードで実行し、stdout を返す。制限時間超過は $null。
function Invoke-Example {
    param([string]$Exe, [string]$Mode, [string]$Path, [int]$Limit)

    $tmpOut = [System.IO.Path]::GetTempFileName()
    $tmpErr = [System.IO.Path]::GetTempFileName()
    $p = Start-Process -FilePath $Exe -ArgumentList "--vm=$Mode", "`"$Path`"" `
        -PassThru -NoNewWindow -RedirectStandardOutput $tmpOut -RedirectStandardError $tmpErr
    $done = $p.WaitForExit($Limit * 1000)
    if (-not $done) {
        try { $p.Kill() } catch {}
        # kill 後もリダイレクト先ハンドルの解放にラグがあるため待ってから削除する
        try { $p.WaitForExit(5000) | Out-Null } catch {}
        Start-Sleep -Milliseconds 200
        Remove-Item $tmpOut, $tmpErr -Force -ErrorAction SilentlyContinue
        return $null
    }
    $out = ''
    foreach ($attempt in 1..10) {
        try { $out = [System.IO.File]::ReadAllText($tmpOut); break }
        catch { Start-Sleep -Milliseconds 200 }
    }
    Remove-Item $tmpOut, $tmpErr -Force -ErrorAction SilentlyContinue
    return $out
}

# 実行ごとに変わる値（ヒープアドレス）を伏せる。`id()` の pointer.value や
# `<Index object at 0x...>` は off/auto に関係なくプロセスごとに変わるため、
# これを残すと VM モード差と区別できない。
function Get-Normalized {
    param([string]$Text)
    $t = $Text -replace '0x[0-9a-fA-F]+', '0xADDR'
    # 10 桁以上の連続数字はアドレス（通常の例題が出す整数はこれより短い）
    $t = $t -replace '\b\d{10,}\b', 'ADDR'
    return $t
}

$same = 0
$diff = 0
$timeout = 0
$diffs = @()

foreach ($f in $examples) {
    $off  = Invoke-Example $exe 'off'  $f.FullName $TimeoutSec
    $auto = Invoke-Example $exe 'auto' $f.FullName $TimeoutSec

    if ($null -eq $off -or $null -eq $auto) {
        $timeout++
        Write-Host "[TIMEOUT] $($f.Name)" -ForegroundColor Yellow
    } elseif ((Get-Normalized $off) -ceq (Get-Normalized $auto)) {
        $same++
    } else {
        $diff++
        $diffs += [PSCustomObject]@{ File = $f.Name; Off = $off; Auto = $auto }
        Write-Host "[DIFF] $($f.Name)" -ForegroundColor Red
    }
}

Write-Host ""
Write-Host "identical: $same   differing: $diff   timeout: $timeout" -ForegroundColor $(if ($diff -eq 0) { 'Green' } else { 'Red' })

foreach ($d in $diffs) {
    Write-Host "`n--- $($d.File) : off ---"
    Write-Host $d.Off
    Write-Host "--- $($d.File) : auto ---"
    Write-Host $d.Auto
}

if ($diff -ne 0) { exit 1 }
