# compare_vm_modes.ps1 — `--vm=off` と `--vm=auto` の出力が byte-identical かを検証する。
#
# バイトコード VM / AST 型解決層（#16）の変更は「解釈経路の観測可能な挙動を変えない」ことが
# 不変条件なので、各変更後にこれを実行して回帰を確認する。
#
# **stdout と stderr の両方**を比較する（#20）。
# 以前は stderr をリダイレクトしながら読み捨てていたため、トレースバックや例外メッセージの
# モード差が検出できなかった（#15d-2 の `<anonymous>` 不一致が 45 例題を素通りした）。
# 併せて `*_error.ar` も比較対象に含める（#20-b）— 実行時エラー系 9 件が
# **トレースバックを実際に踏む唯一の例題群**で、ここが最も差の出やすい経路だから。
# 静的エラー系は型検査が実行前に走るので両モードで自明に一致し、含めても誤検出しない。
#
# 使い方:
#   .\compare_vm_modes.ps1                    # 全例題（_error 含む）を比較
#   .\compare_vm_modes.ps1 -Filter phys       # ファイル名部分一致で絞り込み
#   .\compare_vm_modes.ps1 -SkipErrorExamples # `_error` 例題を除外（旧挙動）

param(
    [string]$Filter = '',
    # examples/bench は経過時間を出力するため off/auto で一致しない（比較対象外が既定）
    [switch]$IncludeBench,
    # 1 例題あたりの上限秒数（超えたら kill して TIMEOUT 扱い）
    [int]$TimeoutSec = 60,
    # `*_error.ar` を比較対象から外す（#20-b の追加分を無効化する退避用）
    [switch]$SkipErrorExamples
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
    $isError = ($name -match '_error' -or $name -match '__errors')
    (-not ($skip -contains $name)) -and
    (-not ($isError -and $SkipErrorExamples)) -and
    ($Filter -eq '' -or $name -like "*$Filter*")
} | Sort-Object Name

# 1 例題を指定 VM モードで実行し、stdout と stderr を返す。制限時間超過は $null。
# 終了コードは見ない（`_error` 例題は非 0 で終わるのが正常）。
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
    # プロセス終了後もハンドル解放にラグがあるため、読めるまで数回リトライする。
    $out = ''
    foreach ($attempt in 1..10) {
        try { $out = [System.IO.File]::ReadAllText($tmpOut); break }
        catch { Start-Sleep -Milliseconds 200 }
    }
    $err = ''
    foreach ($attempt in 1..10) {
        try { $err = [System.IO.File]::ReadAllText($tmpErr); break }
        catch { Start-Sleep -Milliseconds 200 }
    }
    Remove-Item $tmpOut, $tmpErr -Force -ErrorAction SilentlyContinue
    return [PSCustomObject]@{ Out = $out; Err = $err }
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
# stderr を実際に出した例題数。0 なら「stderr 比較が 1 度も発火していない」＝
# 検査に歯が無い状態なので、件数を出して気づけるようにする（#20）。
$withStderr = 0

foreach ($f in $examples) {
    $off  = Invoke-Example $exe 'off'  $f.FullName $TimeoutSec
    $auto = Invoke-Example $exe 'auto' $f.FullName $TimeoutSec

    if ($null -eq $off -or $null -eq $auto) {
        $timeout++
        Write-Host "[TIMEOUT] $($f.Name)" -ForegroundColor Yellow
        continue
    }

    if ($off.Err -ne '' -or $auto.Err -ne '') { $withStderr++ }

    $outSame = (Get-Normalized $off.Out) -ceq (Get-Normalized $auto.Out)
    $errSame = (Get-Normalized $off.Err) -ceq (Get-Normalized $auto.Err)

    if ($outSame -and $errSame) {
        $same++
    } else {
        $diff++
        # どちらのストリームが食い違ったかを出す（traceback 差は stderr にしか出ない）
        $streams = @()
        if (-not $outSame) { $streams += 'stdout' }
        if (-not $errSame) { $streams += 'stderr' }
        $streamLabel = $streams -join '+'
        $diffs += [PSCustomObject]@{
            File = $f.Name; Stream = $streamLabel
            OffOut = $off.Out; AutoOut = $auto.Out
            OffErr = $off.Err; AutoErr = $auto.Err
            OutSame = $outSame; ErrSame = $errSame
        }
        Write-Host "[DIFF:$streamLabel] $($f.Name)" -ForegroundColor Red
    }
}

Write-Host ""
Write-Host "identical: $same   differing: $diff   timeout: $timeout" -ForegroundColor $(if ($diff -eq 0) { 'Green' } else { 'Red' })
Write-Host "examples that produced stderr: $withStderr" -ForegroundColor $(if ($withStderr -eq 0) { 'Yellow' } else { 'DarkGray' })

foreach ($d in $diffs) {
    if (-not $d.OutSame) {
        Write-Host "`n--- $($d.File) : stdout off ---"
        Write-Host $d.OffOut
        Write-Host "--- $($d.File) : stdout auto ---"
        Write-Host $d.AutoOut
    }
    if (-not $d.ErrSame) {
        Write-Host "`n--- $($d.File) : stderr off ---"
        Write-Host $d.OffErr
        Write-Host "--- $($d.File) : stderr auto ---"
        Write-Host $d.AutoErr
    }
}

if ($diff -ne 0) { exit 1 }
