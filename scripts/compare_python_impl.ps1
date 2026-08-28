# compare_python_impl.ps1 — Rust 実装と参照実装（impl_python）の stdout 差分検査（#31）
#
# ── なぜ要るか ──────────────────────────────────────────────────────────────
# compare_vm_modes.ps1 は `--vm=off` と `--vm=on` を比べるので、**両モードが同じ間違いを
# する形**を構造的に検知できない（#27 の `for-target-shadow` で実バグを取り逃した）。
# さらに #33 で `--vm=off` を削除すると compare_vm_modes 自体が成立しなくなる。
# ⇒ 「Rust とは独立に書かれた実装」との突き合わせが、その代わりの網になる。
#
# ── 何を比べるか ────────────────────────────────────────────────────────────
# **stdout だけ**を比べる。エラーメッセージ（stderr）の文言は実装ごとに違うので比較しない
# （Rust は色付きの表・impl_python は 1 行）。Rust 側は **既定モード**（`--vm` を渡さない）で
# 走らせる。#33 で `--vm=off` が消えてもこのスクリプトはそのまま使える。
#
# ── 既知の差分（$knownDiff）────────────────────────────────────────────────
# ⚠ **impl_python は 100 コミット前（33ef765）に同期**されている（`impl_python/__main__.py`
# 冒頭の "git SHA:" を参照）。それ以降に Rust 側へ入った修正・機能はすべて差分になる。
# なので既知差分は**理由つきで明示的に列挙**する。列挙に無い例題は既定で検査対象。
#   - 新しい例題を足すと**自動的に検査される**（合わなければ落ちる）。これは意図した設計で、
#     「例題が無い／検査されない言語機能はゲートに映らない」（#34/#36 の教訓）を避けるため。
#   - 逆に、$knownDiff に載っているのに**一致するようになった**例題は STALE として報告する
#     （黙って残すと網が緩む）。
#
# 使い方:
#   .\scripts\compare_python_impl.ps1                 # 検査（差分があれば exit 1）
#   .\scripts\compare_python_impl.ps1 -ShowSkipped    # 既知差分の一覧も出す
#   .\scripts\compare_python_impl.ps1 -Filter finally # ファイル名部分一致で絞り込み

param(
    [string]$Filter = '',
    [switch]$ShowSkipped,
    [int]$TimeoutSec = 30
)

$ErrorActionPreference = 'Continue'
$repo = Split-Path -Parent $PSScriptRoot   # scripts/ の 1 つ上 = リポジトリ直下
$exe = Join-Path $repo 'target/release/arrow.exe'
if (-not (Test-Path $exe)) { throw "not built: $exe (run: cargo build --release)" }

# 対話入力・GUI・外部プロセス依存（compare_vm_modes.ps1 と同じ方針）
$skip = @(
    'debug_demo', 'async_bench', 'async_demo', 'spider_render', 'spider_solitaire',
    'rs_struct', 'flat_bench', 'flat_bench_interp', 'flat_bench_module',
    'cs_form_app', 'cs_proc_app', 'js_proc_test', 'js_proc_async_test', 'math_render',
    'importation'
)

# 既知差分: 例題名 → 理由。⚠ 理由を書けないものを足さないこと（黙ったスキップは網を殺す）。
$knownDiff = @{
    # (a) impl_python が未対応の言語機能・組み込み（NameError / AttributeError / TypeError を出す）
    'parse_ar'                       = 'py: 組み込み parse_ar 未実装（AST を値として返す・#56 で新設）'
    'parse_ar_error'                 = 'py: 組み込み parse_ar 未実装（#56 で新設）'
    'unregistered_type_call_error'   = 'py: tuple は Python の組み込みなので NameError にならない（#56 で新設）'
    'enum_in_function_error'         = 'py: enum バリアント値の int 検査が無い（str をそのまま通す・#68 で新設）'
    'built_in'                       = 'py: 組み込みの対応範囲が狭い（id/repr 等）'
    'math_string'                    = 'py: m"..." 数式文字列そのものが未実装（ParseError になる・#78 で新設）'
    'builtin_shadow'                 = 'py: 組み込みのシャドウ規則が未実装'
    'collection'                     = 'py: コレクション組み込みの一部が未実装'
    'collection_error'               = 'py: 例外の出し方が違う（未対応の組み込み経由）'
    'fixed_list'                     = 'py: fixed_list 未実装'
    'fixed_list_error'               = 'py: fixed_list 未実装'
    'freeze_collection'              = 'py: freeze の伝播が未実装'
    'attr_access_paths'              = 'py: 属性アクセス経路の一部が未実装'
    'polymorphism_error'             = 'py: エラーの出方が違う（AttributeError で落ちる）'
    'complex_error'                  = 'py: complex の一部演算が未実装'
    'mustbe_error'                   = 'py: mustbe 失敗時の出力形式が違う'
    'raise_span_fields'              = 'py: raise した例外への file/line/col/code_context 焼き込みが未実装（0 / False を返す・#77 で新設）'
    'async_string_share'             = 'py: AsyncManager 未実装'
    'async_closure_share'            = 'py: AsyncManager 未実装'
    'async_vm_body'                  = 'py: AsyncManager 未実装'
    # (b) FFI / 外部言語ブリッジ（外部ツールチェーンに依存し実装差が本質的）
    'bench_ab_cdll'                  = 'py: 計測用（time の再代入が未対応）。#47 の A/B ベンチで意味論の例題ではない'
    'cs_interop_test'                = 'py: C# ブリッジの状態保持が違う'
    'event_cs_fire'                  = 'py: Signal の external_id が未実装'
    'event_external_handler'         = 'py: 外部イベントキューが未実装'
    'ffi_boundary_check'             = 'py: FFI 境界検査が未実装'
    'ffi_boundary_check_error'       = 'py: FFI 境界検査が未実装'
    'ffi_boundary_value_call_error'  = 'py: FFI 境界検査が未実装'
    'import_py_json'                 = 'py: py-int モジュールの値変換が違う'
    # ⚠ impl_python は `import[py]` / `import[py-int]` の束縛自体が未対応
    #    （`cannot assign to immutable variable` になる。`import_py_json` と同じ原因）。
    'import_py_search_path'          = 'py: import[py] の束縛が未対応（#61/#69 で新設）'
    'import_py_int_search_path'      = 'py: import[py-int] の束縛が未対応（#61/#69 で新設）'
    # ⚠ python_converter（`.py` → Arrow AST のソース翻訳）は **Rust 専用機能**で、
    #    impl_python には相当実装が無い（`parser/imports.py` の `lang in ("py","py-int")` は
    #    body を `return []` にし、`interpreter.py` が `importlib.import_module` で
    #    CPython の実行時 import にフォールバックする）。そのため `test_modules.*` が
    #    sys.path に無く `AttributeError: module ... has no attribute ...` になる。
    #    ⇒ 変換器の例題は原理的に一致しない。**実測して理由を確認済み（2026-08-28）**。
    'py_decorators'                  = 'py: python_converter（import[py] のソース翻訳）が Rust 専用（項目20 で新設）'
    'py_decorators_error'            = 'py: 同上。変換時エラーを出さず素通しする（項目20 で新設）'
    'py_kwonly'                      = 'py: python_converter が Rust 専用（項目24 で新設）'
    'py_defaults'                    = 'py: python_converter が Rust 専用（項目1 で新設）'
    'py_defaults_error'              = 'py: 同上。変換時エラーを出さず素通しする（項目1 で新設）'
    'py_reassign'                    = 'py: python_converter が Rust 専用（項目2 で新設）'
    'py_ternary'                     = 'py: python_converter が Rust 専用（項目11 で新設）'
    'py_subscript'                   = 'py: python_converter が Rust 専用（項目3 で新設）'
    'py_slice'                       = 'py: python_converter が Rust 専用（項目4 で新設）'
    'py_membership'                  = 'py: python_converter が Rust 専用（項目12 で新設）'
    'py_identity'                    = 'py: python_converter が Rust 専用（項目13 で新設）'
    'stale_arc_check'                = 'py: .arc を UTF-8 として読んで UnicodeDecodeError'
    'swd_nested_runner'              = 'py: バイナリを UTF-8 として読んで UnicodeDecodeError'
    'typed_abi'                      = 'py: バイナリを UTF-8 として読んで UnicodeDecodeError'
    # (c) 値の表示形式（repr）の違い
    'block_return_typecheck'         = 'py: リスト内の str を引用符なしで表示する'
    'other_typing'                   = 'py: リスト repr と一部の型推論が違う'
    'result'                         = 'py: Result の repr が違う（Ok(5.0) と 5.0）'
    # (d) 同期以降（33ef765..）に Rust 側へ入った意味論の修正 — py が古い
    'copy_method'                    = 'py 古い: mut→let のコピー意味論（#15e で Rust を修正）'
    'mut_to_let_copy'                = 'py 古い: mut→let のコピー意味論（#15e）'
    'variable'                       = 'py 古い: static mut の扱い'
    'block_return_typecheck_error'   = 'py 古い: block_return の実行時型検査が無い（#35 で Rust に追加）'
    'global_assign_from_fn_error'    = 'py: NameError の文言・traceback 形式が違う'
    # (e) 実行時エラーの出力形式（Rust は色付きトレースバック・py は 1 行）
    'runtime_error'                  = 'py: 実行時エラーの出力形式が違う'
    'traceback_frame_names'          = 'py: トレースバックの形式が違う'
    'try_except'                     = 'py: 例外メッセージの形式が違う'
    'try_except_errors'              = 'py: 例外メッセージの形式が違う'
}

$categoryDirs = @('basics', 'collections', 'classes', 'typing', 'exceptions', 'async', 'apps', 'interop', 'repl')

$examples = $categoryDirs | ForEach-Object {
    Get-ChildItem (Join-Path $repo "examples/$_/*.ar") -ErrorAction SilentlyContinue
} | Where-Object {
    $n = $_.BaseName
    (-not ($skip -contains $n)) -and ($Filter -eq '' -or $n -like "*$Filter*")
} | Sort-Object FullName

# 1 プロセスを制限時間つきで走らせて stdout を返す（$null = タイムアウト）。
# ⚠ stdout/stderr は**非同期で同時に読む**。逐次 ReadToEnd はデッドロックする（#38）。
function Invoke-Impl {
    param([string]$File, [string]$Arguments, [int]$Limit)
    $psi = New-Object System.Diagnostics.ProcessStartInfo
    $psi.FileName = $File
    $psi.Arguments = $Arguments
    $psi.WorkingDirectory = $repo
    $psi.UseShellExecute = $false
    $psi.RedirectStandardOutput = $true
    $psi.RedirectStandardError = $true
    $psi.CreateNoWindow = $true
    $p = [System.Diagnostics.Process]::Start($psi)
    $o = $p.StandardOutput.ReadToEndAsync()
    $e = $p.StandardError.ReadToEndAsync()
    if (-not $p.WaitForExit($Limit * 1000)) {
        try { $p.Kill() } catch {}
        return $null
    }
    [void]$e.Result
    return $o.Result
}

function Normalize([string]$s) {
    if ($null -eq $s) { return $null }
    return ($s -replace "`r`n", "`n").TrimEnd()
}

$same = 0; $diff = 0; $skipped = 0; $stale = 0; $timeout = 0
$diffRows = @(); $skipRows = @(); $staleRows = @()

foreach ($ex in $examples) {
    $name = $ex.BaseName
    $rel = ($ex.FullName.Substring($repo.Length + 1)) -replace '\\', '/'
    $known = $knownDiff.ContainsKey($name)

    $rust = Normalize (Invoke-Impl -File $exe -Arguments "`"$($ex.FullName)`"" -Limit $TimeoutSec)
    $py = Normalize (Invoke-Impl -File 'python' -Arguments "-m impl_python `"$rel`"" -Limit $TimeoutSec)

    if ($null -eq $rust -or $null -eq $py) {
        $timeout++
        $diffRows += "TIMEOUT   $rel"
        continue
    }

    if ($rust -eq $py) {
        if ($known) {
            # 既知差分に載っているのに一致した → 一覧から外せる（網を緩めないため報告する）
            $stale++
            $staleRows += ("{0,-46} {1}" -f $rel, $knownDiff[$name])
        } else {
            $same++
        }
        continue
    }

    if ($known) {
        $skipped++
        $skipRows += ("{0,-46} {1}" -f $rel, $knownDiff[$name])
        continue
    }

    $diff++
    $rl = $rust -split "`n"
    $pl = $py -split "`n"
    $hint = ''
    for ($i = 0; $i -lt [Math]::Max($rl.Count, $pl.Count); $i++) {
        $a = if ($i -lt $rl.Count) { $rl[$i] } else { '<none>' }
        $b = if ($i -lt $pl.Count) { $pl[$i] } else { '<none>' }
        if ($a -ne $b) { $hint = "line $($i + 1): rust=[$a] py=[$b]"; break }
    }
    $diffRows += "DIFFER    $rel`n            $hint"
}

Write-Host ''
Write-Host ("checked: {0}   identical: {1}   unexpected diff: {2}   timeout: {3}" -f ($same + $diff + $timeout), $same, $diff, $timeout)
Write-Host ("known diff (skipped): {0}   stale entries: {1}" -f $skipped, $stale)

if ($diffRows.Count -gt 0) {
    Write-Host ''
    Write-Host 'UNEXPECTED DIFFERENCES:' -ForegroundColor Red
    $diffRows | ForEach-Object { Write-Host "  $_" }
}
if ($staleRows.Count -gt 0) {
    Write-Host ''
    Write-Host 'STALE $knownDiff entries (now identical - remove them):' -ForegroundColor Yellow
    $staleRows | ForEach-Object { Write-Host "  $_" }
}
if ($ShowSkipped -and $skipRows.Count -gt 0) {
    Write-Host ''
    Write-Host 'known differences (skipped):' -ForegroundColor DarkGray
    $skipRows | ForEach-Object { Write-Host "  $_" }
}

Write-Host ''
if ($diff -eq 0 -and $timeout -eq 0) {
    Write-Host 'PYTHON-DIFF: clean' -ForegroundColor Green
    exit 0
} else {
    Write-Host 'PYTHON-DIFF: FAILED' -ForegroundColor Red
    exit 1
}
