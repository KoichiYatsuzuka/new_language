# gate_kinds.ps1 — 静的な誤りと警告の種類（TypeErrorKind / TypeWarningKind）の全部に、gate/ の中で期待を持つテストが
# 1 件以上あるかを数える網（gate_speed_plan.md 5-2。type_obligations と同じ形）。
#
# - 種類の一覧は src/type_check/errors.rs の `fn name()`（網羅の match・種類を足すとコンパイルが通らない）から読む。
#   ここに一覧を書くと、種類を足したときに黙って古くなるため。
# - gate/ の `# expect: static <種類>` / `# expect: warning <種類>`（expect-file も）を数える。
# - 届かない種類は下の $unreachable に**理由つきで**載せる（黙って外さない）。載っているのにテストがある・もう無い種類が
#   載っている、は STALE で落とす（一覧が古くなったことを知らせる）。
#
# 使い方: ./scripts/gate_kinds.ps1          # 足りなければ exit 1
# 最後の行は `GATE-KINDS: <種類の数> kinds, <期待のある数> covered, <届かない数> unreachable, <足りない数> missing`。
# ⚠ このファイルは日本語コメントを含むので UTF-8 BOM 付きで保存すること。
param([string]$Root = '')

$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
if (-not $Root) { $Root = Join-Path $repo 'gate' }
. (Join-Path $PSScriptRoot 'gate_judge.ps1')

# ── 届かない種類（理由つき）─────────────────────────────────────────────────
$unreachable = [ordered]@{
    'ProtocolInheritance'      = '宣言と表示（Display）だけがあり、作る場所が src に無い（2026-11-23 に grep で確かめた）'
    'ProtocolSkippedCompile'   = '警告。宣言と表示だけがあり、作る場所が src に無い（同上）'
    'CodeEscapesMetafunction'  = '純粋メタ関数の Code を通常のコードへ持ち出す形は、型検査より前に展開が MetaError（pure metafunction cannot be called from ordinary code）で止める。code: そのものはメタ関数の外では構文の誤り。型検査の側は展開が漏れたときの守り'
    'UnexpandedMemberDecorator' = 'メンバーに残った !装飾子 は展開が先に処理する。型検査で出会うのは展開が漏れたときだけの守り（src/type_check/stmt/check.rs のコメント）'
}

# ── 種類の一覧（errors.rs の name()）─────────────────────────────────────────
$src = [IO.File]::ReadAllText((Join-Path $repo 'src\type_check\errors.rs'), [Text.Encoding]::UTF8)
$kinds = New-Object System.Collections.Generic.SortedSet[string]
foreach ($m in [regex]::Matches($src, 'Self::([A-Za-z]+)(?: \{ \.\. \})? => "([A-Za-z]+)"')) {
    if ($m.Groups[1].Value -ne $m.Groups[2].Value) { throw "name() の腕が種類の名前と違う: $($m.Value)" }
    [void]$kinds.Add($m.Groups[1].Value)
}
if ($kinds.Count -lt 50) { throw "errors.rs から種類を読めなかった（$($kinds.Count) 件）。name() の形が変わったか" }

# ── gate/ の期待 ─────────────────────────────────────────────────────────────
$covered = @{}
foreach ($f in Get-GateTestFiles $Root) {
    foreach ($x in (Read-GateExpectations $f.FullName).Items) {
        if ($x.Stage -eq 'static' -or $x.Stage -eq 'warning') {
            if (-not $covered.ContainsKey($x.Kind)) {
                $covered[$x.Kind] = if ($f.FullName.StartsWith($repo + '\')) { $f.FullName.Substring($repo.Length + 1) } else { $f.FullName }
            }
        }
    }
}

$missing = @(); $stale = @(); $unknown = @()
foreach ($k in $kinds) {
    if ($covered.ContainsKey($k)) {
        if ($unreachable.Contains($k)) { $stale += "$k is listed as unreachable but $($covered[$k]) expects it" }
        continue
    }
    if (-not $unreachable.Contains($k)) { $missing += $k }
}
foreach ($k in $unreachable.Keys) { if (-not $kinds.Contains($k)) { $stale += "$k is listed as unreachable but is not a kind any more" } }
foreach ($k in $covered.Keys) { if (-not $kinds.Contains($k)) { $unknown += "$k (expected in $($covered[$k])) is not a kind" } }

foreach ($k in $missing) { Write-Host "MISSING   $k  (no gate/ test expects it; write one or list it as unreachable with the reason)" -ForegroundColor Red }
foreach ($s in $stale) { Write-Host "STALE     $s" -ForegroundColor Red }
foreach ($u in $unknown) { Write-Host "UNKNOWN   $u" -ForegroundColor Red }
foreach ($k in $unreachable.Keys) { Write-Host ("UNREACHABLE {0}: {1}" -f $k, $unreachable[$k]) -ForegroundColor DarkGray }
$bad = $missing.Count + $stale.Count + $unknown.Count
$color = if ($bad -eq 0) { 'Green' } else { 'Red' }
Write-Host ("GATE-KINDS: {0} kinds, {1} covered, {2} unreachable, {3} missing" -f $kinds.Count, ($kinds | Where-Object { $covered.ContainsKey($_) }).Count, $unreachable.Count, $missing.Count) -ForegroundColor $color
if ($bad -gt 0) { exit 1 } else { exit 0 }
