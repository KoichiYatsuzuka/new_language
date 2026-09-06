# hash_eq_identity.ps1 — **同一性で扱う型の一覧が、ハッシュ側と等値側でずれていないか**を見る。
#
# ── なぜ要るのか ──────────────────────────────────────────────────────────────
# 辞書の健全性は「`values_eq(a, b)` が真なら `default_hash(a) == default_hash(b)`」に依る。
# これが崩れると **「入れたのに引けない辞書」** になる（bug_fix.md B1 の起票文そのもの）。
#
# 「参照の同一性で扱う型」（`Function` / `Generator` / `Signal` …）は、
#   - `src/interpreter/ops/hash.rs`   の `hash_into`    … `ptr_hash(...)`
#   - `src/interpreter/ops/equality.rs` の `values_eq_at` … `Rc::ptr_eq(...)`
# と **2 箇所に同じ一覧**があり、両者は必ず一致していなければならない。
# しかし一致を強制するものが無い。⇒ このスクリプトが唯一の網。
#
# ⚠⚠ **非対称がある**（実測）:
#   - `hash_into`   は `_ =>` を持たない **網羅 match** ⇒ `Value` に種別を足すと**コンパイルエラー**
#   - `values_eq_at` は末尾に **`_ => false`** がある     ⇒ 足しても**黙って「等しくない」**になる
#   つまり「ハッシュ側だけ直して等値側を忘れる」は**コンパイラに止められない**。ここを見る。
#
# ⚠ 関数スコープで区切ること。`equality.rs` には `values_ref_eq`（`Instance`/`Class`/`List`/
#   `Dict`/`Set` を同一性で比べる**別の関数**）があり、ファイル全体を grep すると
#   **5 件の誤警報**になる（実際に一度出した）。
#
# 使い方:
#   ./scripts/hash_eq_identity.ps1
#   ./scripts/hash_eq_identity.ps1 -Verbose    # 一覧も表示する
#
# ⚠ このファイルは日本語コメントを含むので **UTF-8 BOM 付きで保存すること**。
param(
    [switch]$ShowList
)

$ErrorActionPreference = 'Continue'
$repo = Split-Path -Parent $PSScriptRoot

# ── 指定した関数の本体行だけを返す（波括弧の深さで区切る）──────────────────
function Get-FnBody {
    param([string]$Path, [string]$FnName)

    $lines = [System.IO.File]::ReadAllLines($Path)
    $start = -1
    for ($i = 0; $i -lt $lines.Length; $i++) {
        if ($lines[$i] -match ("\bfn\s+" + [regex]::Escape($FnName) + "\s*[(<]")) {
            $start = $i
            break
        }
    }
    if ($start -lt 0) { return $null }

    $body = New-Object 'System.Collections.Generic.List[string]'
    $depth = 0
    $seen = $false
    for ($i = $start; $i -lt $lines.Length; $i++) {
        $l = $lines[$i]
        # 行内コメントを落としてから括弧を数える（`//` の中の `{` に釣られないため）
        $bare = [regex]::Replace($l, '//.*', '')
        $depth += ([regex]::Matches($bare, '\{')).Count
        $depth -= ([regex]::Matches($bare, '\}')).Count
        if ($depth -gt 0) { $seen = $true }
        $body.Add($l)
        if ($seen -and $depth -le 0) { break }
    }
    return $body
}

$hashPath = Join-Path $repo 'src/interpreter/ops/hash.rs'
$eqPath = Join-Path $repo 'src/interpreter/ops/equality.rs'

foreach ($p in @($hashPath, $eqPath)) {
    if (-not (Test-Path $p)) {
        Write-Host "NOT FOUND: $p" -ForegroundColor Red
        exit 2
    }
}

# ── ① ハッシュ側: `Value::X(y) => ptr_hash(` の腕 ────────────────────────────
$hashBody = Get-FnBody -Path $hashPath -FnName 'hash_into'
if ($null -eq $hashBody) {
    Write-Host "hash.rs に fn hash_into が見つからない（改名した？）" -ForegroundColor Red
    exit 2
}
$hashSet = New-Object 'System.Collections.Generic.HashSet[string]'
foreach ($l in $hashBody) {
    $m = [regex]::Match($l, '^\s*Value::([A-Za-z0-9_]+)\(\w+\)\s*=>\s*ptr_hash\(')
    if ($m.Success) { $null = $hashSet.Add($m.Groups[1].Value) }
}

# ── ② 等値側: `(Value::X(a), Value::X(b)) => Rc/Arc::ptr_eq(` の腕 ───────────
$eqBody = Get-FnBody -Path $eqPath -FnName 'values_eq_at'
if ($null -eq $eqBody) {
    Write-Host "equality.rs に fn values_eq_at が見つからない（改名した？）" -ForegroundColor Red
    exit 2
}
$eqSet = New-Object 'System.Collections.Generic.HashSet[string]'
foreach ($l in $eqBody) {
    $m = [regex]::Match($l,
        '^\s*\(Value::([A-Za-z0-9_]+)\(\w+\),\s*Value::([A-Za-z0-9_]+)\(\w+\)\)\s*=>\s*(?:std::sync::)?(?:Rc|Arc)::ptr_eq\(')
    if ($m.Success -and $m.Groups[1].Value -eq $m.Groups[2].Value) {
        $null = $eqSet.Add($m.Groups[1].Value)
    }
}

# ── ③ 負の対照: 抽出が 0 件なら「一致」ではなく**検査が空回りしている** ──────
# ⚠ 正規表現が実装の書き方の変化で当たらなくなると、差分 0 で緑になってしまう。
#   それは「無い検査」より悪い。下限を置いて気づけるようにする。
$MIN = 8
if ($hashSet.Count -lt $MIN -or $eqSet.Count -lt $MIN) {
    Write-Host "検査が空回りしている疑い: hash=$($hashSet.Count) eq=$($eqSet.Count)（下限 $MIN）" -ForegroundColor Red
    Write-Host "  腕の書き方が変わって正規表現が当たらなくなった可能性がある。**緑を信じないこと**。" -ForegroundColor Yellow
    exit 2
}

if ($ShowList) {
    Write-Host "hash_into  で同一性ハッシュ ($($hashSet.Count)):" -ForegroundColor Cyan
    Write-Host ("  " + (($hashSet | Sort-Object) -join ', '))
    Write-Host "values_eq_at で同一性比較 ($($eqSet.Count)):" -ForegroundColor Cyan
    Write-Host ("  " + (($eqSet | Sort-Object) -join ', '))
    Write-Host ""
}

# ── ④ 突き合わせ ─────────────────────────────────────────────────────────────
$onlyHash = @($hashSet | Where-Object { -not $eqSet.Contains($_) } | Sort-Object)
$onlyEq = @($eqSet | Where-Object { -not $hashSet.Contains($_) } | Sort-Object)

Write-Host ("checked: hash_into {0} / values_eq_at {1}" -f $hashSet.Count, $eqSet.Count)

if ($onlyHash.Count -eq 0 -and $onlyEq.Count -eq 0) {
    Write-Host "HASH-EQ-IDENTITY: consistent" -ForegroundColor Green
    exit 0
}

Write-Host ""
Write-Host "同一性で扱う型の一覧がずれている:" -ForegroundColor Red
if ($onlyHash.Count -gt 0) {
    Write-Host '  ハッシュだけ同一性 (等値側が構造比較か `_ => false` に落ちている):' -ForegroundColor Yellow
    foreach ($v in $onlyHash) { Write-Host "      Value::$v" }
    Write-Host "    ⇒ 同じ実体なのに `==` が偽になる／別実体が等しく見える。" -ForegroundColor Yellow
}
if ($onlyEq.Count -gt 0) {
    Write-Host "  等値だけ同一性 (ハッシュ側が構造ハッシュ):" -ForegroundColor Yellow
    foreach ($v in $onlyEq) { Write-Host "      Value::$v" }
    Write-Host "    ⇒ **等しいのにハッシュが違う** ⇒ 入れたのに引けない辞書になる。" -ForegroundColor Yellow
}
Write-Host ""
Write-Host "直し方: どちらの扱いが正しいかを決めて、両方の腕を揃える。" -ForegroundColor Cyan
Write-Host "  hash.rs    : src/interpreter/ops/hash.rs    の fn hash_into" -ForegroundColor Cyan
Write-Host "  equality.rs: src/interpreter/ops/equality.rs の fn values_eq_at" -ForegroundColor Cyan
Write-Host "HASH-EQ-IDENTITY: FAILED" -ForegroundColor Red
exit 1
