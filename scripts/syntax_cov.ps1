# syntax_cov.ps1 — 例題スイートが **一度も書いていない構文** を機械的に数える（#85）。
#
# プラン冒頭が「**未カバーの構文を例題側から数える手段がまだ無い**（`Stmt` の全 variant ×
# 文脈のマトリクスが無い）」と自認していた穴を塞ぐ。そこから出た実バグは #56 / #68 / #71 /
# #75 / #84 の **5 件**で、**全件が「その形の例題が 1 本も無かった」**で通り抜けている。
#
# ⚠⚠ **例題を実行しない**。`AR_SYNTAX_COV=1` はパース直後に AST を歩いて stderr へ出し、
#    **実行せずに終了する**（src/syntax_cov.rs）。GUI 例題も async も FFI も起こさないので
#    タイムアウトも孫プロセスも無く、**決定的**で速い。
#    ⇒ `force_gate.ps1` が 161 例題中 4 本をタイムアウト経由で完走させているのとは対照的。
#
# ⚠ フックは **feature `tw_stats` を付けたビルドにしか存在しない**（既定ビルドではコードごと消える）。
#    このスクリプトが専用ビルドを行う。
#
# ⚠ 母集団（言語に存在する variant 一覧）は **Rust 側が `SyntaxCov[all_stmt]` / `[all_expr]` で出す**。
#    ここに一覧を書くとそちらが黙って古くなるため（#59/#81/#84 が潰してきたドリフト）。
#    観測された種別が母集団に無ければ **STALE POPULATION で落とす**（実データによる母集団検査）。
#
# 使い方:
#   ./scripts/syntax_cov.ps1                 # 未カバーの variant と文脈マトリクスを出す
#   ./scripts/syntax_cov.ps1 -Pairs          # 親>子 のペア表も出す（384 件。既定では出さない）
#
# ⚠ ペア表は**文脈を持たない**（`Match>Let` はあるが「入れ子 fn の中の `Match>Let`」は区別しない）。
#   実測: #84 ①③ はペア表で検出できたが、②（入れ子 fn のブロック式）は `Let>Block` が
#   他の例題で既に埋まっていたため**検出できない**。文脈ごとに分けるとキーが 3 倍以上に
#   増えて読めなくなるので、**意図的に分けていない**（代わりに上の nested-fn 列を見る）。
#   ./scripts/syntax_cov.ps1 -SkipBuild
#   ./scripts/syntax_cov.ps1 -Root examples                    # 今までの例題（既定で archived を除く）
#   ./scripts/syntax_cov.ps1 -Root examples -Exclude 'archived|practical_examples'
#   ⚠ 既定の対象は gate/（gate_speed_plan.md 5-6）。除くもの（-Exclude）の既定は、gate/ なら lib/ と *_mods/、
#     examples/ なら archived/。
#
# ⚠ このファイルは**日本語コメントを含むので UTF-8 BOM 付きで保存すること**（PS5.1 は BOM 無しを ANSI で読む）。
param(
    [int]$Timeout = 30,
    [switch]$SkipBuild,
    [switch]$Pairs,
    [string]$Exclude = '',
    [string]$Root = '',  # 既定は gate/（5-6）。-Root examples で今までの例題
    [int]$Jobs = 0,      # 共通ランナーの並列数（0 = 論理コアの半分・8-1）
    [switch]$Fresh       # 保存した結果を使わず走らせ直す
)

$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot   # scripts/ の 1 つ上 = リポジトリ直下

if (-not $SkipBuild) {
    Write-Host "building with --features tw_stats ..." -ForegroundColor DarkGray
    # cargo は進捗を stderr に出す。`$ErrorActionPreference=Stop` のままだと PS5.1 が
    # それを終了エラー扱いにするので、この呼び出しの間だけ緩める（tw_stats.ps1 と同じ）。
    $prevEap = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    cargo build --features tw_stats | Out-Null
    $ErrorActionPreference = $prevEap
    if ($LASTEXITCODE -ne 0) { throw "build failed" }
}

$exe = Join-Path $repo 'target\debug\arrow.exe'
if (-not (Test-Path $exe)) { throw "not built: $exe" }

if (-not $Root) { $Root = Join-Path $repo 'gate' }
$Root = (Resolve-Path $Root).Path.TrimEnd('\')
if (-not $Exclude) {
    $Exclude = if ($Root -eq (Join-Path $repo 'examples')) { '\\archived\\' } else { '\\lib\\|_mods\\' }
}
$files = @(Get-ChildItem -Path $Root -Filter *.ar -Recurse |
         Where-Object { $_.FullName -notmatch $Exclude } | Sort-Object FullName)

$stmt = @{}; $expr = @{}; $ctx = @{}; $pair = @{}
$allStmt = @(); $allExpr = @()
$ran = 0; $failed = 0
$failedFiles = @()

# 共通ランナー（gate_run.ps1・gate_speed_plan.md 8-1）で走らせる: 並列・出力はファイルへ・結果は exe のハッシュで保存
# （-Fresh で走らせ直す）。以前は 1 本ずつ Start-Process で走らせていた（gate/ で 70 秒）。
# ⚠ **作業ディレクトリを 2 通り試す**。`import` は**エントリスクリプトの dir 基準**だが、
#    `ar_config.json`（`import[rs]` の crates_path 等）は **cwd からの祖先ウォーク**で探す
#    （#69/#74）。片方でしか解決できない例題が実在するので、**測れた方を採る**。
#    ⇒ まず全部をファイルの dir で走らせ、測れなかったものだけをリポジトリ直下で走らせ直す。
. (Join-Path $PSScriptRoot 'gate_run.ps1')
# ⚠ ランナーは時間切れのとき taskkill を呼ぶ。`Stop` のままだと PS5.1 がその stderr を終了エラーにするので緩める。
$prevEap = $ErrorActionPreference
$ErrorActionPreference = 'Continue'
$covEnv = @{ AR_SYNTAX_COV = '1' }
$errByFile = @{}
$first = @(Invoke-GateRun -Exe $exe -Root $Root -Files @($files | ForEach-Object { $_.FullName }) -WorkDir file `
    -ExtraEnv $covEnv -TimeoutSec $Timeout -Jobs $Jobs -Fresh:$Fresh)
$retry = @()
foreach ($r in $first) {
    $errByFile[$r.File] = if ($r.TimedOut) { 'TIMEOUT' } else { Read-GateText $r.ErrPath }
    if ($errByFile[$r.File] -notmatch 'SyntaxCov\[') { $retry += $r.File }
}
if ($retry.Count -gt 0) {
    foreach ($r in @(Invoke-GateRun -Exe $exe -Root $Root -Files $retry -WorkDir $repo -ExtraEnv $covEnv -TimeoutSec $Timeout -Jobs $Jobs -Fresh:$Fresh)) {
        # 以前と同じく、時間切れなら TIMEOUT、そうでなければ 2 回目の stderr を理由に使う
        $errByFile[$r.File] = if ($r.TimedOut) { 'TIMEOUT' } else { Read-GateText $r.ErrPath }
    }
}
$ErrorActionPreference = $prevEap

    foreach ($f in $files) {
        $stderr = $errByFile[$f.FullName]

        if ([string]::IsNullOrEmpty($stderr) -or $stderr -notmatch 'SyntaxCov\[') {
            # パースできない例題（`_error.ar` は**わざと**落ちる）はカバレッジに寄与しない。
            # ⚠⚠ **黙って落とさず理由つきで出す**。「測れていない」と「0 件」を混同すると、
            #    #85 が塞ごうとしている穴（緑の見かけ）をこのツール自身が作ってしまう。
            $reason = (($stderr -split "`n") | Where-Object { $_.Trim() } | Select-Object -First 1)
            if (-not $reason) { $reason = '(no output)' }
            $failed++
            $failedFiles += ("{0}  --  {1}" -f $f.FullName.Replace($repo, '.'), $reason.Trim())
            continue
        }
        $ran++

        foreach ($line in ($stderr -split "`n")) {
            if ($line -match '^SyntaxCov\[(\w+)\] total=\d+ ?(.*)$') {
                $cat = $Matches[1]
                $body = $Matches[2].Trim()
                if ($cat -eq 'all_stmt') { if ($allStmt.Count -eq 0 -and $body) { $allStmt = @($body -split '\s+') }; continue }
                if ($cat -eq 'all_expr') { if ($allExpr.Count -eq 0 -and $body) { $allExpr = @($body -split '\s+') }; continue }
                $tbl = switch ($cat) {
                    'stmt' { $stmt }
                    'expr' { $expr }
                    'ctx'  { $ctx }
                    'pair' { $pair }
                    default { $null }
                }
                if ($null -eq $tbl) { continue }
                if (-not $body) { continue }
                foreach ($kv in ($body -split '\s+')) {
                    if ($kv -match '^(.+)=(\d+)$') {
                        $k = $Matches[1]; $v = [long]$Matches[2]
                        if ($tbl.ContainsKey($k)) { $tbl[$k] += $v } else { $tbl[$k] = $v }
                    }
                }
            }
        }
    }

Write-Host ""
Write-Host "examples parsed: $ran   (not measurable: $failed)" -ForegroundColor Cyan
if ($failed -gt 0) {
    # ⚠ 理由まで出す。`_error` 系（わざと落ちる）と**環境依存で測れないもの**は別物。
    foreach ($x in $failedFiles) { Write-Host ("  - " + $x) -ForegroundColor DarkGray }
}
if ($allStmt.Count -eq 0) { throw "no SyntaxCov[all_stmt] seen - は --features tw_stats のビルドか？" }

# ── 母集団の健全性検査（実データ） ────────────────────────────────────────────
# ⚠⚠ 観測した種別が Rust 側の一覧に無ければ、一覧が古い。**黙って「未カバー」に化ける**
#    ことを防ぐため、ここで明示的に落とす。
$stale = @()
foreach ($k in $stmt.Keys) { if ($allStmt -notcontains $k) { $stale += "stmt:$k" } }
foreach ($k in $expr.Keys) { if ($allExpr -notcontains $k) { $stale += "expr:$k" } }
if ($stale.Count -gt 0) {
    Write-Host ""
    Write-Host "STALE POPULATION: src/syntax_cov.rs の ALL_*_KINDS に無い種別が観測された:" -ForegroundColor Red
    foreach ($x in $stale) { Write-Host "  $x" -ForegroundColor Red }
    throw "population list is stale"
}

function Show-Uncovered($label, $all, $seen) {
    $missing = @($all | Where-Object { -not $seen.ContainsKey($_) })
    Write-Host ""
    Write-Host ("=== {0}: {1} / {2} covered ===" -f $label, ($all.Count - $missing.Count), $all.Count) -ForegroundColor Cyan
    if ($missing.Count -eq 0) { Write-Host "  (all covered)" -ForegroundColor Green; return }
    foreach ($m in $missing) { Write-Host ("  UNCOVERED  " + $m) -ForegroundColor Yellow }
}

Show-Uncovered 'Stmt variants' $allStmt $stmt
Show-Uncovered 'Expr variants' $allExpr $expr

# ── 文脈マトリクス ───────────────────────────────────────────────────────────
# ⚠⚠ **ここが #85 の本体**。「最上位には書かれているが関数内・入れ子関数内では 0」を見つける。
#    #68（関数本体の enum）・#75 / #84（入れ子 fn の中）はすべてこの形だった。
$frames = @('top', 'fn', 'nested_fn', 'type', 'module', 'async')
Write-Host ""
Write-Host "=== context matrix (O=written / .=never written / +e = also inside an expression body) ===" -ForegroundColor Cyan
Write-Host ("  {0,-20} {1}" -f 'variant', (($frames | ForEach-Object { '{0,-11}' -f $_ }) -join ''))
foreach ($k in ($allStmt + $allExpr)) {
    if (-not $stmt.ContainsKey($k) -and -not $expr.ContainsKey($k)) { continue }  # 未カバーは上の表で既出
    $cells = foreach ($fr in $frames) {
        $plain = $ctx.ContainsKey("$k@$fr")
        $inExp = $ctx.ContainsKey("$k@$fr+expr")
        $c = if ($plain) { 'O' } else { '.' }
        $e = if ($inExp) { '+e' } else { '  ' }
        '{0,-11}' -f "$c$e"
    }
    Write-Host ("  {0,-20} {1}" -f $k, ($cells -join ''))
}

# ── 入れ子関数の穴（最も見落とされる文脈）────────────────────────────────────
# ⚠⚠ **歴史的な実バグ 5 件のうち 4 件（#68 / #75 / #84 の 3 件）がこの列で起きた**。
#    「最上位や関数直下では書かれているのに、入れ子 fn の中では 1 度も書かれていない」構文を
#    名指しで出す。⇒ 例題を足す先の**優先順位**がそのまま出る。
# ⚠ 母集団は「**最上位 fn の本体には書かれている**構文」に絞る。関数本体に書ける構文は
#    入れ子 fn の本体にも書けるので、この絞り込みは健全（かつ `Field` のようにクラス本体
#    専用のものを弾ける）。絞らないと 40 件出て**読まれない一覧**になる。
$nestedGaps = @()
foreach ($k in ($allStmt + $allExpr)) {
    $inFn     = $ctx.ContainsKey("$k@fn")        -or $ctx.ContainsKey("$k@fn+expr")
    $inNested = $ctx.ContainsKey("$k@nested_fn") -or $ctx.ContainsKey("$k@nested_fn+expr")
    if ($inFn -and -not $inNested) { $nestedGaps += $k }
}
Write-Host ""
Write-Host ("=== written in a top-level fn body, but NEVER inside a nested fn ({0}) ===" -f $nestedGaps.Count) -ForegroundColor Cyan
Write-Host "  (4 of the 5 historical real bugs lived exactly here: #68 / #75 / #84 x2)" -ForegroundColor DarkGray
if ($nestedGaps.Count -eq 0) { Write-Host "  (none)" -ForegroundColor Green }
foreach ($g in $nestedGaps) { Write-Host ("  NESTED-GAP  " + $g) -ForegroundColor Yellow }

if ($Pairs) {
    Write-Host ""
    Write-Host ("=== observed parent>child pairs ({0}) ===" -f $pair.Count) -ForegroundColor Cyan
    $pair.GetEnumerator() | Sort-Object -Property Name | ForEach-Object {
        Write-Host ("  {0,-40} {1}" -f $_.Key, $_.Value)
    }
}

Write-Host ""
Write-Host "SYNTAX-COV-DONE"
