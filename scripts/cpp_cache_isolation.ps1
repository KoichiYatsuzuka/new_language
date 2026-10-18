# cpp_cache_isolation.ps1 — A/B 比較で 2 つの arrow.exe が cpp-lib の中継 DLL のキャッシュを取り合わないようにする（ffi_value_exchange_plan.md 7-34）。
#
# ── なぜ要るか ──────────────────────────────────────────────────────────────
# `import[cpp-lib]` は中継 DLL・シム・印（`ar_{stem}.dll` / `ar_{stem}_shim.*` / `ar_{stem}.syms`）をヘッダの隣に作り、
# 次からはそれを使う（印は生成するコードのハッシュ）。基準の exe と新しい exe は生成するコードが違うことがあるので、
# 交互に走らせると**そのたびに作り直し**、作り直しの表示（`CppBridge: … is stale … rebuilding` やコンパイラの出力）が
# 出力の差になった。負の対照（同じ exe 同士）でも、直前のゲートが別の exe の印で残したキャッシュを最初の 1 回だけ
# 作り直すので差になった。さらに最後に走った exe の印でキャッシュが残るので、コミットする中継 DLL が基準の exe のもの
# になりえた（7-13 / 7-28 で踏んだ）。
#
# ── 何をするか ──────────────────────────────────────────────────────────────
#   1. 始める前のキャッシュを控える
#   2. exe ごとに、控えたキャッシュから始めて cpp-lib を使う例題を 1 回ずつ走らせ（出力は捨てる・作り直しはここで済ませる）、
#      その exe のキャッシュを控える
#   3. 比較では、走らせる exe のキャッシュに入れ替えてから走らせる（2 つの exe のキャッシュが同じなら入れ替えない）
#   4. 終わったら始める前のキャッシュに戻す（ゲートは作業ツリーを変えない）
# ⚠ 例題の外（`examples/DxLib`・`practical_examples`・`archived`）のキャッシュは扱わない（比較の対象外）。
# ⚠ ウォームアップに漏れたヘッダ（cpp-lib を使う例題を `import` 経由でしか読まないもの）は比較の中で作り直されうる。
#   そのときは `Test-CppCacheUnchanged` が知らせる。
#
# 使い方（dot-source する）:
#   . (Join-Path $PSScriptRoot 'cpp_cache_isolation.ps1')
#   $cache = Initialize-CppCacheIsolation -Repo $repo -Exes @($A, $B) -Examples $files -Tmp $tmp -TimeoutSec 60
#   try { ... Use-CppCache $cache $A; <A を走らせる>; Use-CppCache $cache $B; <B を走らせる> ... }
#   finally { Close-CppCacheIsolation $cache }
# ⚠ このファイルは日本語コメントを含むので UTF-8 BOM 付きで保存すること。

# 例題の下の cpp-lib のキャッシュのファイル（無いものも含む・`ar_{stem}.syms` ごとに 5 つ）。
function Get-CppCacheFiles([string]$Repo) {
    $files = @()
    Get-ChildItem (Join-Path $Repo 'examples') -Recurse -Filter 'ar_*.syms' -ErrorAction SilentlyContinue |
        Where-Object { $_.FullName -notmatch '[\\/](DxLib|practical_examples|archived)[\\/]' } |
        ForEach-Object {
            foreach ($suffix in '.dll', '.syms', '_shim.dll', '_shim.exp', '_shim.lib') {
                $files += (Join-Path $_.DirectoryName ($_.BaseName + $suffix))
            }
        }
    return @($files | Sort-Object -Unique)
}

function Get-CppCacheKey([string]$File) { return ($File -replace '[:\\/]', '_') }

# `Files` の今の状態を `Dir` へ控える（無いファイルは控えにも無い）。
function Save-CppCache([string[]]$Files, [string]$Dir) {
    if (Test-Path $Dir) { Remove-Item -Recurse -Force $Dir }
    $null = New-Item -ItemType Directory -Force $Dir
    foreach ($f in $Files) {
        # ⚠ 写せなければ止める（控えが欠けたまま戻すと、控えに無いファイルとして消してしまう・実際に踏んだ）
        if (Test-Path $f) { Copy-Item $f (Join-Path $Dir (Get-CppCacheKey $f)) -Force -ErrorAction Stop }
    }
}

# `Dir` の控えの状態に戻す（控えに無いファイルは消す）。
function Restore-CppCache([string[]]$Files, [string]$Dir) {
    foreach ($f in $Files) {
        $src = Join-Path $Dir (Get-CppCacheKey $f)
        if (Test-Path $src) { Copy-Item $src $f -Force }
        elseif (Test-Path $f) { Remove-Item $f -Force }
    }
}

# 控えの中身の指紋（ファイル名と SHA-256）。2 つの exe のキャッシュが同じかを見る。
function Get-CppCacheFingerprint([string]$Dir) {
    return ((Get-ChildItem $Dir -File | Sort-Object Name | ForEach-Object {
        $_.Name + '=' + (Get-FileHash $_.FullName -Algorithm SHA256).Hash
    }) -join ';')
}

function Initialize-CppCacheIsolation {
    param(
        [Parameter(Mandatory=$true)][string]$Repo,
        [Parameter(Mandatory=$true)][string[]]$Exes,
        [Parameter(Mandatory=$true)][string[]]$Examples,
        [Parameter(Mandatory=$true)][string]$Tmp,
        [int]$TimeoutSec = 60
    )
    $root = Join-Path $Tmp 'cppcache'
    $state = @{ Files = @(Get-CppCacheFiles $Repo); Orig = (Join-Path $root 'orig'); OrigComplete = $false
                Dirs = @{}; Current = $null; Shared = $false }
    # 控えが揃ってから戻す側に回る（揃わなければ何も触らずに止まる）
    Save-CppCache $state.Files $state.Orig
    $state.OrigComplete = $true
    # cpp-lib を使う例題（`import[cpp-lib]` を書いているもの）
    $warm = @($Examples | Where-Object { (Get-Content $_ -Raw -Encoding UTF8) -match 'import\[cpp-lib\]' })
    $i = 0
    try {
        foreach ($exe in @($Exes | Select-Object -Unique)) {
            $i++
            Restore-CppCache $state.Files $state.Orig
            foreach ($w in $warm) {
                $p = Start-Process -FilePath $exe -ArgumentList ('"' + $w + '"') -WorkingDirectory $Repo -NoNewWindow -PassThru `
                    -RedirectStandardOutput (Join-Path $Tmp 'cppcache_warm.out') -RedirectStandardError (Join-Path $Tmp 'cppcache_warm.err')
                if (-not $p.WaitForExit($TimeoutSec * 1000)) { try { $p.Kill() } catch {} }
            }
            # ウォームアップで初めてできたキャッシュも扱う（始める前は無かった → 終わったら消す）
            $state.Files = @(($state.Files + (Get-CppCacheFiles $Repo)) | Sort-Object -Unique)
            $dir = Join-Path $root "exe$i"
            Save-CppCache $state.Files $dir
            $state.Dirs[$exe] = $dir
        }
    } catch {
        Restore-CppCache $state.Files $state.Orig
        throw
    }
    $dirs = @($state.Dirs.Values)
    $state.Shared = ($dirs.Count -le 1) -or ((Get-CppCacheFingerprint $dirs[0]) -eq (Get-CppCacheFingerprint $dirs[1]))
    if ($state.Shared) { Restore-CppCache $state.Files $dirs[0] }
    Write-Host ("cpp-lib caches: {0} file(s), warmed with {1} example(s); the two exes {2}" -f `
        $state.Files.Count, $warm.Count, $(if ($state.Shared) { 'share the same caches' } else { 'get their own caches' })) -ForegroundColor DarkGray
    return $state
}

# 走らせる exe のキャッシュに入れ替える（同じキャッシュなら何もしない）。
function Use-CppCache($State, [string]$Exe) {
    if ($State.Shared -or ($State.Current -eq $Exe)) { return }
    Restore-CppCache $State.Files $State.Dirs[$Exe]
    $State.Current = $Exe
}

# 比較のあいだにキャッシュが作り直されていないか（ウォームアップに漏れたヘッダ）。作り直されていれば警告を出す。
function Test-CppCacheUnchanged($State) {
    $exe = if ($State.Current) { $State.Current } else { @($State.Dirs.Keys)[0] }
    if (-not $exe) { return }
    $now = Join-Path (Split-Path $State.Orig) 'now'
    Save-CppCache $State.Files $now
    if ((Get-CppCacheFingerprint $now) -ne (Get-CppCacheFingerprint $State.Dirs[$exe])) {
        Write-Host 'WARNING: a cpp-lib cache was rebuilt during the comparison (an example the warm-up did not run uses it)' -ForegroundColor Yellow
    }
}

# 始める前のキャッシュに戻す。
function Close-CppCacheIsolation($State) {
    if (($null -eq $State) -or (-not $State.OrigComplete)) { return }
    Restore-CppCache $State.Files $State.Orig
}
