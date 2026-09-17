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
    'field_type_error'               = 'py: フィールド代入の静的型検査が無い（実行時 TypeError になる・0-2 で新設）'
    'return_type_error'              = 'py: return の戻り値型検査が無い（そのまま実行してしまう・0-3 で新設）'
    'method_arg_type_error'          = 'py: メソッド引数の静的型検査が無い（そのまま実行してしまう・0-4 で新設）'
    'ctor_arg_type_error'            = 'py: コンストラクタ引数の静的型検査が無い（そのまま実行してしまう・0-5 で新設）'
    'var_annotation_error'           = 'py: let/mut の型注釈が検査されない（注釈を捨てて実行してしまう・0-1 で新設）'
    'generic_type_ann'               = 'py: 具体化済みジェネリクスの型注釈が未対応（テンプレート実体化の意味論ごと違う・A-2 で新設）'
    'intersection_result_bind'       = 'py: Result の表示形式が違う（py は Ok(5) ではなく 5 を出す・タスク 1.1 で新設）'
    'intersection_result_bind_error' = 'py: Intersection 注釈と右辺の照合が無い（妥当性検査だけして素通しする・タスク 1.1 で新設）'
    'static_mut_assign'              = 'py: static mut のインスタンス経由アクセスが未対応（AttributeError になる・タスク 1.2 で新設）'
    'static_mut_assign_error'        = 'py: static mut への代入の型検査が無い（そのまま実行してしまう・タスク 1.2 で新設）'
    'protocol_in_container'          = 'py: protocol 型の束縛・メンバー解決が未実装（NameError/AttributeError になる・タスク 1.3 で新設）'
    'protocol_in_container_error'    = 'py: 容器の内側の protocol 適合検査が無い（そのまま実行してしまう・タスク 1.3 で新設）'
    'template_result_type_error'     = 'py: テンプレート実体化の結果型が無い（実行時の演算エラーになる・タスク 2.1 で新設）'
    'function_value_type_error'      = 'py: 関数値の型と分散規則の検査が無い（そのまま実行してしまう・タスク 2.2 で新設）'
    'enum_member_type'               = 'py: const クラス変数のクラス名経由アクセスが未対応（AttributeError になる・タスク 2.3 で新設）'
    'enum_member_type_error'         = 'py: enum .value の静的型が無い（int を str 変数へ入れて実行してしまう・タスク 2.3 で新設）'
    'and_or_result_type_error'       = 'py: and/or の結果型検査が無い（bool 変数に int を入れて実行してしまう・タスク 2.5 で新設）'
    'collection_elem_synthesis'      = 'py: list 内の str の表示形式が違う（py は引用符を付けない・タスク 2.7 で新設）'
    'collection_elem_synthesis_error' = 'py: 混在リテラルの要素型合成が無い（list[int] へ混在を入れて実行してしまう・タスク 2.7 で新設）'
    'except_bind_and_match_expr'     = 'py: raise した例外への line 焼き込みが未実装（0 を返す・raise_span_fields と同根・タスク 2.8 で新設）'
    'except_bind_and_match_expr_error' = 'py: except as の束縛型と組み込み例外のフィールド型が無い（str を int 変数へ入れて実行してしまう・タスク 2.8 で新設）'
    'validity_checks'                = 'py: テンプレート型変数に対する is ガードが未対応（0 を返す・タスク 3.4 で新設）'
    'validity_checks_error'          = 'py: 型ガードの型名の存在検査が無い（死んだ腕のまま実行してしまう・タスク 3.4 で新設）'
    'validity_checks_enum_error'     = 'py: enum バリアント値の静的検査が無い（未呼出関数内を見逃す・タスク 3.4 で新設）'
    'upcast_only'                    = 'py: list 内の str の表示形式が違う（py は引用符を付けない・タスク 4.1 で新設）'
    'upcast_only_error'              = 'py: 素の list から list[int] へのダウンキャスト検査が無い（そのまま実行してしまう・タスク 4.1 で新設）'
    'arith_operand_check'            = 'py: str % int（書式化）が未実装（TypeError になる・タスク 4.4 で新設）'
    'arith_operand_check_error'      = 'py: 算術の被演算子の静的検査が無い（実行時の TypeError になる・タスク 4.4 で新設）'
    'user_cast_site_error'           = 'py: __cast__ の受理地点を絞る検査が無い（int 変数に Conv が居座ったまま実行してしまう・タスク 4.3 で新設）'
    'compound_assign_two_stage'      = 'py: str %= int（書式化）が未実装（TypeError になる・arith_operand_check と同じ理由・タスク 5.1 で新設）'
    'compound_assign_two_stage_error' = 'py: 複合代入の結果型の検査が無い（Vec 変数に str が居座ったまま実行してしまう・タスク 5.1 で新設）'
    'block_expr_value_static'        = 'py: list 内の str の表示形式が違う（py は引用符を付けない・upcast_only と同じ理由・タスク 5.2 で新設）'
    'subscript_static'               = 'py: dict 内の str の表示形式が違う（py は引用符を付けない・タスク 5.2b で新設）'
    'collection_method_args'         = 'py: list/set 内の str の表示形式が違う（py は引用符を付けない・タスク 5.2c で新設）'
    'collection_method_args_error'   = 'py: 可変長引数の要素型検査が無い（型が混ざったまま実行してしまう・タスク 5.2c で新設）'
    'new_type_ctor_arg'              = 'py: new_type の連鎖（Kg: Meters: float）が未実装で Kg(None) になる・タスク 5.6 で新設'
    'runtime_type_predicates'        = 'py: 実行時型判定の統一（uint と int の相互許容）が未実装で x is int が False になる・タスク 6.1 で新設'
    'overload_arg_types'             = 'py: オーバーロードの実行時ディスパッチが型を見ない（最初の定義を呼ぶので > int:hi になる）・タスク 5.7 で新設'
    'overload_arg_types_error'       = 'py: オーバーロードの実引数型による解決が無い（黙って実行してしまう・タスク 5.7 で新設）'
    'member_existence'               = 'py: protocol のフィールド要求を method として誤判定する（type Impl does not satisfy protocol HasN・impl_python 側の既知の限界）'
    'cross_type_equality'            = 'py: Option[T] / is None の扱いが未実装（タスク 7.6 で新設）'
    'literal_element_check_error'    = 'py: リテラル要素の個別照合が無い（黙って実行してしまう・タスク 7.7 で新設）'
    'cross_type_equality_error'      = 'py: 異型の等値比較の静的検査が無い（False を返して実行してしまう・タスク 7.6 で新設）'
    'member_existence_error'         = 'py: メンバー存在検査が無い（実行時 AttributeError になる・タスク 7.5 で新設）'
    'cast_mustbe_possible_error'     = 'py: 成功しえない cast / mustbe の静的検査が無い（実行時エラーになる・タスク 7.3 で新設）'
    'let_binding_writes_error'       = 'py: let 束縛のフィールド代入の静的検査が無い（実行時エラーになる・タスク 7.2 で新設）'
    'known_but_wrong_error'          = 'py: 呼び出し可否・反復可否・単項演算子・except の型の静的検査が無い（実行時エラーになる・タスク 7.1 で新設）'
    'new_type_ctor_arg_error'        = 'py: new_type のコンストラクタ引数の型検査が無い（黙って str を包んで実行してしまう・タスク 5.6 で新設）'
    'condition_must_be_bool_error'   = 'py: 条件の bool 厳密検査が無い（真偽性で解釈して実行してしまう・タスク 5.5 で新設）'
    'match_case_pattern_type_error'  = 'py: 死ぬ case 腕の検査が無い（黙って miss になる・タスク 5.4 で新設）'
    'static_mut_class_assign_error'  = 'py: クラス名経由の static mut 代入の型検査が無い（黙って実行してしまう・タスク 5.3 で新設）'
    'trait_field_assign_static_error' = 'py: trait 修飾フィールド代入の静的検査が無い（実行時エラーになる・タスク 5.2d で新設）'
    'subscript_static_error'         = 'py: 添字代入の要素型検査が無い（list[int] に str が黙って入ったまま実行してしまう・タスク 5.2b で新設）'
    'block_expr_value_static_error'  = 'py: block_return の値と ->T の静的照合が無い（実行時エラーになる・タスク 5.2 で新設）'
    'trait_conformance'              = 'py: trait のデフォルト実装の継承が未実装（AttributeError になる・0-10 で新設）'
    'class_virtual_method_error'     = 'py: クラス本体の仮想メソッド禁止が未実装（None を返して実行してしまう・0-10 で新設）'
    'trait_field_redeclare_error'    = 'py: trait フィールド再宣言の禁止が未実装（実行時 TypeError になる・0-9 で新設）'
    'protocol_sites'                 = 'py: protocol 型の束縛・メンバー解決が未実装（NameError/AttributeError になる・0-7 で新設）'
    'template_instantiate_error'      = 'py: テンプレート実体化の引数型検査が無い（そのまま実行してしまう・0-6 で新設）'
    'template_type_param'            = 'py: テンプレート実体化クラスのメモ化が無い（クラスレベル初期化子が毎回走る）＋ float 昇格が未実装（Phase T で新設）'
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
    'py_set'                         = 'py: python_converter が Rust 専用（項目22 で新設）'
    'py_tuple'                       = 'py: python_converter が Rust 専用（項目18 で新設）'
    'py_classvar'                    = 'py: python_converter が Rust 専用（項目5 で新設）'
    'py_inherit'                     = 'py: python_converter が Rust 専用（クラス継承サポートで新設）'
    'py_inherit_error'               = 'py: 同上。変換時エラーを出さず素通しする'
    'py_fstring'                     = 'py: python_converter が Rust 専用（項目19 で新設）'
    'py_fstring_error'               = 'py: 同上。変換時エラーを出さず素通しする（項目19 で新設）'
    'py_comprehension'               = 'py: python_converter が Rust 専用（項目17 で新設）'
    'py_comprehension_error'         = 'py: 同上。変換時エラーを出さず素通しする（項目17 で新設）'
    'py_ellipsis'                    = 'py: python_converter が Rust 専用（項目21 で新設）'
    'py_varargs'                     = 'py: python_converter が Rust 専用（項目6・7 で新設）'
    'py_chained_compare'             = 'py: python_converter が Rust 専用（項目16 で新設）'
    'stale_arc_check'                = 'py: .arc を UTF-8 として読んで UnicodeDecodeError'
    'swd_nested_runner'              = 'py: バイナリを UTF-8 として読んで UnicodeDecodeError'
    'typed_abi'                      = 'py: バイナリを UTF-8 として読んで UnicodeDecodeError'
    # (c) 値の表示形式（repr）の違い
    'block_return_typecheck'         = 'py: リスト内の str を引用符なしで表示する'
    'other_typing'                   = 'py: リスト repr と一部の型推論が違う'
    'result'                         = 'py: Result の repr が違う（Ok(5.0) と 5.0）'
    # ⚠ 内包表記は Arrow の**新しいネイティブ構文**（2026-08-28）。impl_python は同期点が古く
    #    パーサに入っていないので `ParseError: expected `RBRACKET`, got `FOR`` になる。
    #    ⇒ ユーザー方針により impl_python は触らない（同期時の積み残し）。**実測して確認済み**。
    'comprehension'                  = 'py: 内包表記の構文が未実装（Rust 側で 2026-08-28 に追加）'
    # ⚠ B1-a（辞書キー）/ B2-a（list・dict の ==）で新設。**実測して確認済み**。
    #    ユーザー方針により impl_python は触らない（同期時の積み残し）。
    'dict_key_types'                 = 'py 古い: list をキーにすると引けない（実測 `KeyError: key not found: [3, 4]`）。B1-c で Rust は全型をキーにできる'
    # ⚠ B2-b（等値の二層化）で新設。**実測して確認済み**。
    #    ユーザー方針により impl_python は触らない（同期時の積み残し）。
    'equality_numeric_promotion'     = 'py 古い: uint の昇格が無く `uint(3) == 3` が False（実測 4 行目）'
    'eq_dunder_consistency'          = 'py 古い: set の add/remove が __eq__ を見ない（実測 len が 2/3 になり remove が KeyError）'
    'class_identity_across_threads'  = 'py: AsyncManager そのものが未実装（実測 NameError）'
    # ⚠ B1-c（辞書の __hash__ / __eq__ ディスパッチ）で新設。**実測して確認済み**。
    'dict_key_dunder'                = 'py 古い: 辞書が __hash__ / __eq__ を見ない（実測 len が 3 になり、続く索引が KeyError）'
    # ⚠ B5（list / tuple の + と *）で新設。**実測して確認済み**。
    #    py は list + list / list * int は対応しているが tuple + tuple が無い。
    'list_concat_repeat'             = 'py 古い: tuple + tuple が未対応（実測 RuntimeError）。加えてリスト内の str を引用符なしで表示する'
    'list_concat_repeat_error'       = 'py: 実行時エラーの出力形式が違う（Rust は色付きトレースバック・py は 1 行）'
    # ⚠ B6（モジュール本体からの自己呼び出し）で新設。**実測して確認済み**。
    #    py は自己呼び出し自体は正しく動く（差分は repr だけ）＝ Rust の修正が参照実装と一致した証拠。
    'module_selfcall'                = 'py: リスト内の str を引用符なしで表示する（実測 `[hi 0, hi 1, hi 2]`）。自己呼び出し自体は py も動く'
    'module_async_body'              = 'py: AsyncManager そのものが未実装（実測 NameError）'
    # ⚠ B9（Set / Tuple の複製）で新設。**実測して確認済み**。py にも同じバグがある
    #    （代入で set が共有され `let s = {1}; mut t = s; t.add(9)` で s が {1, 9} になる）。
    'set_tuple_copy_semantics'       = 'py 古い: set が代入で共有される（実測 let の s が {1, 9} になる）。Rust は複製する'
    # (d) 同期以降（33ef765..）に Rust 側へ入った意味論の修正 — py が古い
    'copy_method'                    = 'py 古い: mut→let のコピー意味論（#15e で Rust を修正）'
    'mut_to_let_copy'                = 'py 古い: mut→let のコピー意味論（#15e）'
    'variable'                       = 'py 古い: static mut の扱い'
    'block_return_typecheck_error'   = 'py 古い: block_return の実行時型検査が無い（#35 で Rust に追加）'
    'global_assign_from_fn_error'    = 'py: NameError の文言・traceback 形式が違う'
    # ⚠ B1-a: ハッシュ不可能なキーを **py はまだ黙って捨てる**（Rust は TypeError）。
    #    実測: `d[(1,2)] = "x"` の後の行まで実行され「ここには到達しない」が出る＝まさに B1 の症状。
    'dict_key_types_error'           = 'py 古い: None キーを黙って受け入れる（実測: 次の行の「ここには到達しない」が出る）。Rust は仕様で禁止'
    # ⚠ B2-a: py の `==` には**同一性の高速パスが無い**ので、循環リストの `a == a` の時点で
    #    再帰上限に達する（実測 1 行目で `RuntimeError: maximum recursion depth exceeded`）。
    #    Rust は `Rc::ptr_eq` で即 True を返し、別々の循環でのみ RecursionError にする。
    # ⚠ L4（格納時のディープコピー）で循環が作れなくなり、循環前提の例題を
    #    深い入れ子の例題へ書き換えた。py は格納が共有のままなので `a.append(a)` が
    #    循環し、印字の時点で再帰上限に達する（実測）。
    'equality_depth_limit_error'     = 'py 古い: 格納が共有なので a.append(a) が循環し、印字で RecursionError（実測）。Rust は格納時に複製するので循環しない'
    'store_copy_semantics'           = 'py 古い: コンテナ・フィールドへの格納が共有される（実測 let の a が [1, 9] になる）。Rust は複製する'
    # ⚠ L2 / L3 / B11（let の不変性）で新設。**実測して確認済み**。
    'let_immutability'               = 'py 古い: 式から let への束縛が共有される（実測 let item が [1, 9] になる）。Rust は複製する'
    'let_immutability_error'         = 'py: mut パラメータへ let を渡す静的検査が無く、素通りして 1 行目から出力が出る（Rust は StaticTypeError で停止）'
    # ⚠ B4 規則 1（for ターゲットの再束縛禁止）で新設。**実測して確認済み**。
    'for_target_shadow_error'        = 'py: for ターゲットが外側の束縛を覆う静的検査が無く、素通りして 1 行目から出力が出る（Rust は StaticTypeError で停止）'
    'for_target_scope_error'         = 'py: 両方とも NameError になるが**出力形式が違う**（実測：py は stdout へ RuntimeError: NameError: name ... を 1 行、Rust はトレースバック）'
    # ⚠ B10 系統（再帰の深さ上限）で新設。**実測して確認済み**。
    'recursion_limit'                = 'py 古い: 再帰を Python のスタックに任せており RuntimeError になる。Rust は catchable な RecursionError なので except で受けて実行を続けられる'
    'recursion_limit_error'          = 'py 古い: 同上。py は stdout へ RuntimeError を 1 行、Rust は RecursionError のトレースバック'
    # ⚠ B13（コルーチン化）の安全網として新設。**実測して確認済み**。
    'generator_reentrancy'           = 'py 古い: 同じジェネレータ実体を入れ子で回すと py は [] を返すが、**CPython の正解は [[1, 2]]**（実測済み）。Rust が正しい側'
    # ⚠ B13（コルーチン化）で新設。**実測して確認済み**。py は先行評価のまま。
    'generator_lazy'                 = 'py 古い: ジェネレータが先行評価なので無限ジェネレータ（while True: yield）で止まらない。Rust は次の yield までで中断する'
    'generator_lazy_error'           = 'py 古い: 生成時に本体が走るので holder[0] がまだ無く IndexError。Rust は消費時に走るので ValueError: generator is already executing'
    'generator_closure'              = 'py 古い: ジェネレータのクロージャ化に未対応で、かつ先行評価なので§5 の無限ジェネレータで止まらない（B13 段階 E）'
    'yield_placement_error'          = 'py: `yield` の置き場所の静的検査が無く、素通りして 1 行目から出力が出る（Rust は StaticTypeError で停止・B13）'
    # ⚠ フェーズ 8（要素型を捨てる経路を塞ぐ）で新設。**実測して確認済み**。
    'elem_type_kept_error'           = 'py: 型注釈の要素型を検査しないので素通りして「ここには到達しない」まで出る（Rust は StaticTypeError で停止・8.2）'
    'bare_tuple_annotation_error'    = 'py: 型注釈を捨てるので let t: tuple = 1 が通り「ここには到達しない」まで出る（Rust は StaticTypeError で停止・8.3）'
    'bare_tuple_annotation'          = 'py: タプル内の str を引用符なしで表示する（実測 `(1, a)`）。list_concat_repeat と同じ repr の差で、タプルの受け渡し自体は py も動く'
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
