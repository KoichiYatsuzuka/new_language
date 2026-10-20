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
    'js_proc_values', 'js_proc_container_return_error', 'js_proc_instance_arg_error',
    'rs_values', 'rs_unsupported_signature_error', 'rs_u64_overflow_error', 'rs_native_arg_check', 'rs_collections', 'rs_slice_arg_error',
    'importation'
)

# 既知差分: 例題名 → 理由。⚠ 理由を書けないものを足さないこと（黙ったスキップは網を殺す）。
$knownDiff = @{
    # (a) impl_python が未対応の言語機能・組み込み（NameError / AttributeError / TypeError を出す）
    'metafn_expansion_print'         = 'py: メタ関数を字句解析できないので ParseError（3-4 で新設。Rust 側の stdout は "runtime output" と "done" の 2 行だけで、展開時の print は stderr へ出る）'
    'template_constrained_error'     = 'Rust: 制約付きテンプレート・宣言より前の具体化も単相化して検査し StaticTypeError（2-14 で新設）。py: テンプレートのメソッド呼び出しを検査しないので実行してしまう'
    'template_module'                = 'Rust: import したテンプレートの具体化をモジュールの中に置くので、メソッドがモジュールの関数を引ける・同名テンプレートを取り違えない（2-15 で新設）。py: 実行時の具体化のメソッドがモジュールの名前を引けず NameError'
    'template_contexts'              = 'Rust: 閉包・クラスの const・非同期タスク・モジュールの最上位での具体化（10-5 で新設）。py: クラスの const を属性として読めず AttributeError'
    'metafn_type_values'             = 'Rust: ^ の .type などが型の値を返し、list[t] で合成できる（D23・4-2 で新設）。py: メタ関数を展開しないので何も出さない'
    'tuple_subscript'                = 'Rust: 添字の中の , は組になる（d[x, y] は d[(x, y)]・4-2 で新設）。py: この構文を読めず何も出さない'
    'import_namespace'               = 'Rust と py で名前空間の分離は一致（6 行目まで同じ）。py: AsyncManager が無いので非同期の節で NameError'
    'import_namespace_error'         = 'どちらも NameError（修飾なしのモジュールの名前）。py は失敗を stdout に出し、Rust は stderr に出す'
    'builtin_args'                   = 'py: enumerate の start を無視して 0 から数える（実行時の差。タプルの中の文字列の表示は 7-23 で揃えた。Rust 側の引数の静的検査は builtins.ars の宣言から・2026-10-01）'
    'builtin_args_error'             = 'Rust: 組み込み関数の引数（個数・キーワード引数の名前・型）を builtins.ars の仮引数で検査して StaticTypeError。py: 組み込みの引数を静的に検査しないので実行してしまう'
    'builtin_return_types'           = 'py: 実行時の `f is FileObject` が False（Rust は True）で raise し、組み込みの `getenv` も無い（NameError）。Rust 側は builtins.ars の宣言から組み込みの戻り値の型を取る（2026-09-30）'
    'builtin_return_types_error'     = 'Rust: 組み込みの戻り値の型（open → FileObject・repr / getenv → str）と組み込みの列挙・FileObject のメソッド（builtins.ars）で StaticTypeError。py: それらを知らず、getenv も無いので NameError で止まる'
    'template_method_error'          = 'Rust: 単相化した Box[int] のメソッド呼び出しを具体型で検査して StaticTypeError（2026-09-26 新設・フェーズ10 の発端）。py: テンプレートのメソッド呼び出しを検査しないので実行して wrong を出す'
    'template_type_param_error'      = 'Rust: 具体化した本体（Cell[str] の self.v = 0・conv[float] の return n）も StaticTypeError（2026-09-26 に Section 3/4 を追加）。py: 具体化した本体を検査しないので実行して 3 を出す'
    'template_recursion_error'       = 'Rust: 展開時の単相化が再帰的な具体化を止めて MetaError（2-9 で新設）。py: 実行時に呼ばれたときだけ具体化するので、再帰が n == 0 で止まり 3 を出す'
    'private_method'                 = 'Rust: private メソッド・メソッドの中の入れ子の関数・静的メソッド・メソッドの中の非同期タスクで private に届く（10-6 で新設・6 行目まで同じ）。py: AsyncManager が無いので非同期の節で NameError'
    'private_method_error'           = 'Rust: クラスの外から private メソッドを呼ぶと StaticTypeError（10-6 で新設）。py: メソッドのアクセス指定を検査しないので実行して audit: 3 / audit: 5 を出す'
    'gen_element_type_error'         = 'Rust: gen の呼び出しの結果が generator[int] になり、for の変数を str へ入れると StaticTypeError（10-7 で新設）。py: ジェネレータの要素型を検査しないので実行して 1 / 2 を出す'
    'is_template_instance'           = 'Rust: x is Stack[int] を型式として読み、単相化したクラスの具体化の名前で判定する（10-10 で新設）。py: is の後ろの [ を読めず ParseError（何も出さない）'
    'operator_overload_operand'      = 'Rust: __mul__ の多重定義（int / float）を実引数の型で選ぶ（10-13 で新設・2 行目まで同じ）。py: 型で選ばず float 版を int に使ってフィールドの型違いで落ちる'
    'operator_overload_operand_error' = 'Rust: 演算子メソッドの仮引数に合わない右辺を StaticTypeError（10-13 で新設）。py: 検査せず実行して __add__ の中の other.cents で AttributeError'
    'builtin_members_error'          = 'Rust: 組み込みの値の属性の読み・int / float のメソッドを StaticTypeError（10-14 で新設）。py: 検査せず実行して AttributeError'
    'module_mut_state'               = 'Rust: 名前空間の mut のメンバーはモジュールの大域の今の値を読み、外から代入もできる（10-11・enum_member_type_plan.md 6-2）。py: 名前空間が import 時の写しのままで c.count が 0 のまま・名前空間への代入も未対応'
    'unresolved_cpp_header_error'    = 'Rust: 見つからないヘッダの import[cpp-lib] を構文解析の誤りにする（editor_import_resolution_plan.md 1-2）。py: 実行時に CppImport の RuntimeError を stdout に出す'
    'cpp_dll_struct_by_value'        = 'Rust: 構造体を値で渡す・返す C 関数を呼べる（ffi_value_exchange_plan.md 1-3）。py: C のブリッジが値渡し・値返しの構造体を扱わず、その関数が無いという AttributeError を stdout に出す'
    'cpp_lib_struct_by_value'        = 'Rust: 構造体を値で渡す・返す C 関数を呼べる（cpp-lib・ffi_value_exchange_plan.md 1-3）。py: 値渡し・値返しの構造体を扱わず AttributeError を stdout に出す'
    'cpp_struct_by_value_layout_error' = 'どちらもエラーで止まる。Rust はレイアウト不明の構造体を値で受け渡す関数を生成せずエラーを stderr に出し（1-3）、py は AttributeError を stdout に出す'
    'cpp_cstring_nul_error'          = 'Rust: NUL を含む str を const char* へ渡すと ValueError（ffi_value_exchange_plan.md 1-2）。py: 黙って NUL で切れて 1 を返す（直す前の Rust と同じ壊れ方）'
    'py_int_values'                  = 'Rust: py-int で set ↔ set を変換し、i64 に入らない int と Decimal を Python のオブジェクトのまま渡す（ffi_value_exchange_plan.md 2-6）。py: Arrow の set を内部の TlSet のまま Python へ渡すので、型の検査で ValueError'
    'py_int_instance_arg_error'      = 'Rust: Arrow のインスタンスを Python へ渡すと TypeError（型名はクラス名・2-6）。py: 内部の TlInstance のまま渡して通る'
    'py_int_bytes'                   = 'Rust: py-int で Python の bytes / bytearray / memoryview と Arrow の bytes を受け渡す（ffi_value_exchange_plan.md 5-3）。py: bytes は受け渡すが、bytearray を bytes にせずそのまま返すので check が raise する'
    'py_int_classes'                 = 'Rust: py-int の Python のクラスのコンストラクタの値はそのクラスの型で、注釈のあるメンバーに型が付く（ffi_value_exchange_plan.md 7-27）。py: `is` の右辺のモジュールで修飾した型（`pc.Counter`）を構文解析できず ParseError'
    'py_int_classes_error'           = 'Rust: py-int のクラスのメンバーを静的に検査して StaticTypeError（7-27）。py: そのインスタンスのメソッドを呼べず AttributeError を stdout に出す'
    'py_int_functions'               = 'Rust: py-int の関数の注釈のある引数・戻り値に型が付き、float の引数は int も受ける（ffi_value_exchange_plan.md 7-30）。py: Arrow の list を Python の list に変換せず内部の TlList のまま渡すので len() が TypeError'
    'py_int_functions_error'         = 'Rust: py-int の関数の引数の数・型の誤りは StaticTypeError（7-30）。py: 静的に検査せず Python の TypeError を stdout に出す'
    'foreign_ctor_args'              = 'Rust: 外部言語のクラスのコンストラクタの引数を型スタブの __init__ で検査する（ffi_value_exchange_plan.md 7-31）。py: C のブリッジが値渡しの構造体の関数（fp_pair_sum）を中継せず AttributeError'
    'cpp_os_mismatch_error'          = 'Rust: ar_config.json の "os" が動いている OS と違えば import の誤り（ffi_value_exchange_plan.md 7-32）。py: OS の設定を読まず、隣に DLL が無いという RuntimeError を stdout に出す'
    'bytes_basics'                   = 'Rust: 符号化の誤りは CPython と同じ UnicodeDecodeError の文言（ffi_value_exchange_plan.md の encoding.rs と python_builtins_plan.md 3-1 の ops/codecs.rs を 1 つにした）。py: 旧文言（cannot decode bytes as ..）'
    'bytes_decode_error'             = 'どちらも止まる（3-1）。Rust は UnicodeDecodeError（ValueError の派生）、py は ValueError。py は捕まえていない例外を stdout に出し、Rust は stderr に出す'
    'bytes_str_mix_error'            = 'Rust: bytes と str を混ぜると StaticTypeError（3-1）。py: 静的に検査せず、実行時の TypeError を stdout に出す'
    'file_bytes_list_error'          = 'どちらも止まる。Rust はバイト単位のファイルへ list を書く行が StaticTypeError（ffi_value_exchange_plan.md 7-11）。py: 静的に検査せず、実行時の TypeError を stdout に出す'
    'cpp_cstring_arg'                = 'Rust: const char* の引数は bytes（ffi_value_exchange_plan.md 4-3・D-13）。py: C のスタブが const char* を str と宣言するので、None（NULL）を渡す行が StaticTypeError'
    'cpp_cstring_str_arg_error'      = 'Rust: const char* へ str を渡すと StaticTypeError（4-3・D-13）。py: str を UTF-8 で渡して 3 を出す（4-3 より前の Rust と同じ）'
    'cpp_raw_pointers'               = 'Rust: char* の戻り値は bytes、ほかのポインタは借用の Ptr（4-3）。py: C のスタブがポインタを int / mut と宣言するので StaticTypeError'
    'cpp_ptr_protocol'               = 'Rust: ArPtrProtocol の値返しを Option[Ptr[void]] で受け取り展開・解放する（ffi_value_exchange_plan.md 4-5）。py: C のスタブがポインタの引数を mut と宣言するので StaticTypeError'
    'cpp_ptr_protocol_release_error' = 'Rust: 解放関数も STATIC も無い ArPtrProtocol は ValueError（4-5・D-15）。py: その関数を中継しないので AttributeError を stdout に出す'
    'cpp_struct_expand'              = 'Rust: 値返し・値渡し・配列の構造体を展開する（ffi_value_exchange_plan.md 4-6）。py: 値返しの構造体の関数を中継せず AttributeError'
    'cpp_struct_list_error'          = 'Rust: ArPtrProtocol のフィールドを持つ構造体の配列の to_list() は TypeError（4-6・7-14）。py: その関数を中継しないので AttributeError を stdout に出す'
    'cpp_default_args'               = 'Rust: C の既定値（DEFAULTPARAM( = x )）の引数を省略すると宣言した既定値が入る（ffi_value_exchange_plan.md 7-13）。py: C のブリッジがその関数（既定値を持つ関数）を中継せず AttributeError を stdout に出す'
    'cpp_default_args_error'         = 'Rust: 読めない既定値（マクロ名）の引数の省略は StaticTypeError（7-13）。py: その関数を中継せず AttributeError を stdout に出す'
    'cpp_lib_included_header_error'  = 'Rust: 取り込んだヘッダの関数も型スタブに入り、str の引数は StaticTypeError（ffi_value_exchange_plan.md 7-28）。py: 静的に検査せず、実行時の TypeError を stdout に出す'
    'cpp_callback_param_error'       = 'Rust: 関数ポインタ（callback）の引数を持つ C 関数は型スタブに入り、使うと理由つきの StaticTypeError（ffi_value_exchange_plan.md 7-29）。py: その関数を中継せず AttributeError を stdout に出す'
    'cpp_list_to_c'                  = 'Rust: list / bytes を ArPtrProtocol として渡し、出力バッファと list へ書き戻す（ffi_value_exchange_plan.md 4-7）。py: list を C の配列（ArPtrProtocol）として渡さないので、fp_sum64 の結果が合わず check が raise する'
    'cpp_typed_pointers'             = 'Rust: 型つき経路で bool・bytes・Ptr・ArPtrProtocol を受け渡す（ffi_value_exchange_plan.md 4-4）。py: C のスタブがポインタの引数を mut と宣言するので StaticTypeError'
    'cpp_ptr_protocol_arg_error'     = 'Rust: ArPtrProtocol の引数へ str / int / dict を渡すと StaticTypeError（4-4・4-7）。py: その関数を中継しないので AttributeError を stdout に出す'
    'cpp_native_arg_check'           = 'Rust: 素の function 型を通して C の数値・bool の引数へ数値でない値を渡すと TypeError、C の関数は呼ばない（ffi_value_exchange_plan.md 7-12）。py: function 型の仮引数へ具体的な関数型を渡せず StaticTypeError'
    'cpp_native_arg_error'           = 'Rust: C の int の引数へ str を渡すと TypeError（7-12）。py: function 型の仮引数へ具体的な関数型を渡せず StaticTypeError'
    'cs_proc_values'                 = 'Rust: C# の値を型スタブの宣言どおりに受け渡す（nullable 注釈・CsObject・ffi_value_exchange_plan.md 2-1）。py: import[cs-proc] で `cs_probe/cs_probe.dll`（パッケージのディレクトリ）を探さず ParseError'
    'cs_proc_utf8'                   = 'Rust: Arrow から C# へ非 ASCII の文字列を送る（ffi_value_exchange_plan.md 2-2）。py: cs_probe/cs_probe.dll を見つけられず ParseError（cs_proc_values と同じ）'
    'cs_proc_collections'            = 'Rust: cs-proc でコレクションと bytes をコピーで受け渡す（ffi_value_exchange_plan.md 5-2）。py: cs_probe/cs_probe.dll を見つけられず ParseError（cs_proc_values と同じ）'
    'cs_dll_exception_error'         = 'Rust: C# の例外を Arrow のエラーにして止まる（stderr・ffi_value_exchange_plan.md 5-1）。py: 同じエラーで止まるが、捕まえていない例外を stdout に出す'
    'cs_dll_collections'             = 'Rust: cs-dll でバイト列・ポインタ・数値の配列・list を受け渡す（ffi_value_exchange_plan.md 5-5）。py: bytes / Ptr / 配列を cs-dll の境界で変換せず、.NET 側のアクセス違反でプロセスごと落ちる'
    'async_native_call'              = 'Rust: 非同期タスクの中から C の関数を呼ぶ（ffi_value_exchange_plan.md 6-1）。py: AsyncManager が無く NameError'
    'async_builtin_len'              = 'Rust: 非同期タスクの本体で組み込みの len を使う（ffi_value_exchange_plan.md 7-18）。py: AsyncManager が無く NameError'
    'async_cs_calls'                 = 'Rust: 非同期タスクの中から C# を呼ぶ（ffi_value_exchange_plan.md 6-2）。py: cs_probe/cs_probe.dll を見つけられず ParseError（cs_proc_values と同じ）'
    'async_thread_bound'             = 'Rust: スレッドに縛られる値はタスクへ切り離した複製を送る（ffi_value_exchange_plan.md 7-19）。py: AsyncManager が無く NameError'
    'async_thread_bound_error'       = 'Rust: メインの Signal をタスクの中で使うと RuntimeError（7-19）。py: AsyncManager が無く NameError'
    'async_closure_mut'              = 'Rust: タスクの本体で本体の mut 変数を捕まえる入れ子の関数（ffi_value_exchange_plan.md 7-22）。py: AsyncManager が無く NameError'
    'foreign_error_catch'            = 'Rust: 外部言語の例外を ForeignError・FfiTypeError で捕まえる（ffi_value_exchange_plan.md 7-4）。py: cs_probe/cs_probe.dll を見つけられず ParseError（cs_proc_values と同じ）'
    'ffi_int_overflow'               = 'Rust: py-int / js-proc から int に入らない整数が返ると OverflowError（ffi_value_exchange_plan.md 7-9）。py: js-proc に対応しておらず ParseError'
    'py_int_overflow_error'          = 'Rust: `-> int` の Python の関数が 2**70 を返すと OverflowError（7-9）。py: 境界の検査が無く int に 64 ビットの上限も無いので値をそのまま出す'
    'py_int_element_types'           = 'Rust: py-int の型スタブが注釈の要素型を残す（ffi_value_exchange_plan.md 7-8）。py: Python の set を変換せず（型 unknown）`in` で RuntimeError'
    'py_int_element_type_error'      = 'Rust: 注釈の要素型に反する戻り値は FfiTypeError（7-8）。py: 境界の検査が無く値をそのまま出す'
    'py_int_foreign_error'           = 'Rust: py-int で Python が投げた例外は ForeignError（ffi_value_exchange_plan.md 7-24）。py: 関数の例外は揃えたが、Python のオブジェクトのメソッド呼び出しに対応していない'
    'py_int_exception_error'         = 'Rust: py-int で Python が投げた例外を捕まえないと ForeignError（7-24）。py: 捕まえない例外を stdout に Unhandled exception: … と出す（実行時エラーの出力形式の差）'
    'cs_dll_array_arg_error'         = 'Rust: cs-dll の配列・list・IntPtr の引数へ合わない値を渡すと StaticTypeError（5-5）。py: 型スタブの型が違うので検査されず呼んで C# 側の例外'
    'cs_proc_null_contract_error'    = 'Rust: null にならないはずの C# の戻り値の null を TypeError にする（2-1）。py: 上と同じく DLL を見つけられず ParseError'
    'cs_proc_namespaced_type'        = 'Rust: 外部言語のスタブのクラスを別名つきの型名 cp.Person で書ける（ffi_value_exchange_plan.md 7-6）。py: cs_probe/cs_probe.dll を見つけられず ParseError（cs_proc_values と同じ）'
    'cpp_dll_int_widths'             = 'Rust: C の整数型を幅と符号どおりに受け渡す（ffi_value_exchange_plan.md 1-6）。py: unsigned int の最大値を -1 として読み、例題の検査が ValueError で止まる（直す前の Rust と同じ）'
    'cpp_lib_int_widths'             = 'Rust: C の整数型を幅と符号どおりに受け渡す（cpp-lib・1-6）。py: long を返す関数などを関数一覧に入れず AttributeError で止まる'
    'cpp_u64_overflow_error'         = 'Rust: 2^63 以上の unsigned long long を OverflowError にする（1-6）。py: 黙って -1 にする（直す前の Rust と同じ壊れ方）'
    'cpp_struct_ptr_arg_error'       ='Rust: 構造体ポインタにインスタンス以外を渡すと TypeError（1-2）。py: 整数をそのままポインタとして渡してアクセス違反（直す前の Rust は segfault）'
    'unresolved_py_int_error'        = 'Rust: 型の出所（.pyi / .py）が無い import[py-int] を構文解析の誤りにする（editor_import_resolution_plan.md 1-2）。py: 型を検査せず実行して最後まで進む'
    'module_global_assign_error'     = 'Rust: import したモジュールの let への代入・中身の書き換え・mut への型の合わない代入を StaticTypeError（enum_member_type_plan.md 6-1 で新設）。py: 静的に検査せず実行して g.LABEL = の AttributeError で止まる'
    'template_as_value'              = 'Rust: Stack[int].empty() / Counter[T].n を具体化として読む（10-9 で新設）。py: 添字として読み not subscriptable で落ちる'
    'template_as_value_error'        = 'Rust: 具体化を値として使った結果を型検査して StaticTypeError（10-9 で新設）。py: 添字として読み not subscriptable で落ちる'
    'name_resolution_error'          = 'Rust: どこにも無い名前を StaticTypeError（10-12 で新設）。py: 検査せず実行して最初の NameError で止まる'
    'module_types'                   = 'Rust: モジュールの型をモジュール名の名前空間で扱い、型注釈に t.Tag と書ける（10-8 で新設）。py: 型注釈の t.Tag を読めず ParseError（何も出さない）'
    'class_in_function_error'        = 'Rust: 関数の中の class は ParseError（10-16 で新設）。py: 検査せず関数の中の class を実行して unreachable を出す'
    'py_class_factory'               = 'Rust: Python の関数の中の class を実行する（10-16 で新設）。py: import[py] したモジュールの count_by を引けず AttributeError'
    'py_class_factory_error'         = 'Rust: 自分で定義したクラスを返す Python の関数を Arrow から使うと StaticTypeError（10-16 で新設）。py: 検査せず、モジュールの属性も引けず AttributeError'
    'py_import_in_function_error'    = 'Rust: Python の関数の中の import は変換の誤り（10-16 で新設）。py: 検査せず、モジュールの属性も引けず AttributeError'
    'py_nested_class'                = 'Rust: Python のクラス本体の中のクラスを持ち上げて変換する（10-18 で新設）。py: import[py] したモジュールの summary を引けず AttributeError'
    'py_class_body_error'            = 'Rust: Python のクラス本体の変換しない文（if）を変換の誤りにする（10-18 で新設）。py: 検査せず、モジュールの属性も引けず AttributeError'
    'except_exception'               = 'Rust: Exception をすべての例外の基底にし、except Exception が組み込みの例外も利用者の例外も捕まえる（10-19 で新設）。py: 何も出さない'
    'except_error_trait_error'       = 'Rust: except Error を静的エラーにし except Exception を促す（10-19 で新設）。py: 検査せず Error で捕まえて caught を出す'
    'py_module_types'                = 'Rust: import[py] のクラスをモジュール名の名前空間で扱い、is / 型注釈に z.Dog と書ける・基底でも当たる（python_builtins_plan.md の 6 節で新設）。py: 型注釈の z.Animal を読めず ParseError（何も出さない）'
    'float_floor_div_mod'            = 'Rust: float の // と % を CPython の _float_div_mod で計算し、1.0 // 0.0 を ZeroDivisionError で捕まえる（タスク 1-2 で新設・16 行目まで同じ）。py: Python の ZeroDivisionError が Arrow の例外にならず RuntimeError で止まる'
    'float_floor_div_mod_error'      = 'Rust: 7.5 % 0.0 を ZeroDivisionError: float modulo（CPython 3.12 の文言）で止める（タスク 1-2 で新設）。py: RuntimeError: ZeroDivisionError: modulo by zero'
    'seq_ordering'                   = 'Rust: list 同士・tuple 同士の大小比較（タスク 1-3 で新設）。py: 静的に cannot compare で止まる（何も出さない）'
    'dunder_iter'                    = 'Rust: 辞書とジェネレータの __iter__（タスク 1-4 で新設）。py: dict に __iter__ が無く AttributeError で止まる'
    'complex'                        = 'Rust: complex を CPython の repr と同じく (1+2j) / 3j と表示する（タスク 1-5 で変更）。py: 成分に .0 を付けた (0.0+3.0j) のまま'
    'known_but_wrong'                = 'Rust: complex を CPython の repr と同じく (-1-2j) と表示する（タスク 1-5 で変更・4 行目まで同じ）。py: (-1.0-2.0j)'
    'float_repr'                     = 'Rust: float を CPython の repr と同じく表示する（タスク 1-5 で新設・8 行目まで同じ）。py: inf / nan に .0 を付けて inf.0 / nan.0 と出す'
    'py_builtin_shadow'              = 'Rust: Python のモジュールの最上位で組み込みの名前を束縛し直せる（タスク 1-6 で新設）。py: import[py] したモジュールの read_back を引けず AttributeError'
    'builtin_values'                 = 'Rust: 組み込みの名前を値として使い、list / int を <class ''list''> / <class ''int''> と表示する（CPython と同じ・タスク 1-7 で新設）。py: <built-in function list> と出す'
    'builtin_values_error'           = 'Rust: list(5) を実行時の TypeError で止め、stdout には何も出さない（タスク 1-7 で新設）。py: RuntimeError: TypeError を stdout に出す'
    'py_isinstance'                  = 'Rust: isinstance（タスク 2-1 で新設）。py: import[py] したモジュールの属性を引けず AttributeError'
    'py_isinstance_error'            = 'Rust: isinstance(3, len) を CPython と同じ TypeError で止める（タスク 2-1 で新設）。py: 検査せず False を出す'
    'py_type'                        = 'Rust: type(x)（タスク 2-2 で新設）。py: import[py] したモジュールの属性を引けず AttributeError'
    'py_type_error'                  = 'Rust: 3 引数の type() を TypeError で止める（タスク 2-2 で新設）。py: 検査せず str を出す'
    'py_getattr'                     = 'Rust: getattr / hasattr / setattr（タスク 2-3 で新設）。py: import[py] したモジュールの属性を引けず AttributeError'
    'py_getattr_error'               = 'Rust: setattr で let のインスタンスを書き換える形を TypeError で止める（タスク 2-3 で新設）。py: setattr が無く NameError'
    'py_introspect'                  = 'Rust: issubclass / callable とフェーズ 2 の組み込みをまとめて使う（タスク 2-4 で新設）。py: import[py] したモジュールの属性を引けず AttributeError'
    'py_introspect_error'            = 'Rust: issubclass の第 1 引数がインスタンスなら TypeError（タスク 2-4 で新設）。py: issubclass が無く NameError'
    'dict_constructor'               = 'Rust: dict(..) を CPython と同じく呼べる（タスク 3-1 で新設）。py: 鍵の文字列を引用符なしで {a: 3, b: 2} と表示する'
    'dict_constructor_error'         = 'Rust: dict(5) を CPython と同じ TypeError で止める（タスク 3-1 で新設）。py: 検査せず {} を出す'
    'py_aggregates'                  = 'Rust: all / any / min / max / sum / sorted ...（タスク 3-2 で新設・5 行目まで同じ）。py: min / max の key= を見ず pear banana と出し、default= を受けず ValueError で止まる'
    'py_aggregates_error'            = 'Rust: max([]) を実行時の ValueError で止め、stdout には何も出さない（タスク 3-2 で新設）。py: RuntimeError: ValueError を stdout に出す'
    'py_conversions'                 = 'Rust: map / filter / iter / hash / ord / chr / hex / oct / bin / ascii（タスク 3-3 で新設）。py: map が無く NameError'
    'exception_hierarchy'            = 'Rust: 組み込みの例外が CPython の階層を持つ（タスク 4-1 で新設）。py: 辞書の引き損ないが Arrow の例外にならず Unhandled exception で止まる'
    'exception_hierarchy_error'      = 'Rust: let e: KeyError = LookupError(..) を静的に止める（タスク 4-1 で新設）。py: LookupError が無く NameError'
    'exception_args'                 = 'Rust: 組み込みの例外の args と CPython の str(e) / repr(e)（タスク 4-2 で新設）。py: 例外に args が無く AttributeError'
    'exception_args_error'           = 'Rust: ValueError(message=..) を CPython と同じ TypeError で止める（タスク 4-2 で新設）。py: キーワード引数を受けて <ValueError object> を出す'
    'py_exception_args'              = 'Rust: Python の例外の args / str(e) / super().__init__（タスク 4-2 で新設）。py: import[py] したモジュールの describe_all を引けず AttributeError'
    'except_qualified'               = 'Rust: except ae.AppError: の修飾名で捕まえる（タスク 4-3 で新設）。py: except の修飾名を構文が受けず ParseError（何も出さない）'
    'py_qualified_except'            = 'Rust: Python の except h.ConfigError: と Arrow の except h.ConfigError:（タスク 4-3 で新設）。py: except の修飾名を構文が受けず ParseError（何も出さない）'
    'py_except_tuple'                = 'Rust: Python の except (A, B):（タスク 4-4 で新設）。py: import[py] したモジュールの parse を引けず AttributeError'
    'print_keywords_error'           = 'Rust: print("x", end=5) を CPython と同じ TypeError で止める（タスク 5-1 で新設）。py: 検査せず x を出す'
    'py_open'                        = 'Rust: open を Python の形（"w" / "a" / "rb" / "x"・encoding=）で呼ぶ（タスク 5-2 で新設）。py: mode "a" を受けず ValueError。interop の同名例題は import[py] したモジュールの save を引けず AttributeError'
    'py_unsupported_builtin_error'   = 'Rust: Arrow に無い組み込み（format / exit）を参照する Python のモジュールを変換時に誤りにする（タスク 1-1 で新設）。py: import[py] したモジュールの show を引けず AttributeError'
    'freeze_like_let'                = 'Rust: freeze x を let x = x と同じ 1 実装にし、関数の中でも動く（タスク 1-9 で新設）。py: __freeze__ を呼ばず frozen at を出さない'
    'freeze_like_let_error'          = 'Rust: クロージャが捕まえた変数の freeze を実行時の TypeError で止め、stdout には何も出さない（タスク 1-9 で新設）。py: RuntimeError: TypeError を stdout に出す'
    'bound_method'                   = 'Rust: obj.method を値として読むと受け手を束縛した関数になる（タスク 1-8 で新設）。py: 束縛せず、静的に get takes 1 argument(s) but 0 were given で止まる（何も出さない）'
    'py_bound_method'                = 'Rust: Python の f = s.apply / getattr / map(s.apply, ..)（タスク 1-8 で新設）。py: import[py] したモジュールの run を引けず AttributeError'
    'bound_method_more'              = 'Rust: gen メソッド・組み込みの値のメソッドの束縛と束縛メソッドの型（タスク 1-10 で新設・5 行目まで同じ）。py: 辞書の鍵の文字列を引用符なしで (a, 1) と表示する'
    'bound_method_more_error'        = 'Rust: 不変の list の append を束縛して呼ぶ形を静的に止める（タスク 1-10 で新設）。py: 検査せず [1, 2, 3] を出す'
    'callable_field'                 = 'Rust: 関数を入れたフィールドを self.f(x) で呼ぶと、属性を読んでその値を呼ぶ（タスク 1-11 で新設）。py: メソッドとしてしか引かず AttributeError: no method f'
    'callable_field_error'           = 'Rust: 呼べない値を入れたフィールドを呼ぶ形を静的に止める（タスク 1-11 で新設）。py: 検査せず AttributeError: no method n'
    'descriptor'                     = 'Rust: クラス側の値の __get__ / __set__ が読み・代入で働く（ディスクリプタのプロトコル・タスク 1-12 で新設）。py: 値そのもの <Fahrenheit object> を返し、代入は has no field で止まる'
    'descriptor_error'               = 'Rust: __set__ の値の型と合わない代入を静的に止める（タスク 1-12 で新設）。py: 検査せず has no field fahrenheit で止まる'
    'py_property'                    = 'Rust: Python の @property / @x.setter / @Base.x.getter が組み込みの property で動く（タスク 6-1 で新設・CPython と同じ出力）。py: import[py] したモジュールの run を引けず AttributeError'
    'bytes'                          = 'Rust: 不変のバイト列 bytes（添字は int・encode / decode・CPython と同じ誤りの文言・タスク 3-1 で新設・CPython と同じ出力）。py: str.encode を持たず AttributeError'
    'bytes_error'                    = 'Rust: bytes + str を静的に止める（タスク 3-1 で新設）。py: str.encode を持たず AttributeError'
    'py_class_body_names'            = 'Rust: クラス本体の名前を参照するクラス属性・メソッドの別名・クラス属性のラムダ（タスク 6-3 で新設・CPython と同じ出力）。py: import[py] したモジュールの run を引けず AttributeError'
    'py_bytes'                       ='Rust: Python の b".." リテラルが組み込みの bytes に変換される（タスク 3-1 で新設・CPython と同じ出力）。py: import[py] したモジュールの run を引けず AttributeError'
    'frozenset'                      ='Rust: 不変でハッシュできる集合 frozenset（演算子・set との比較・辞書のキー・タスク 3-1 で新設・CPython と同じ出力）。py: frozenset を知らず NameError'
    'frozenset_error'                = 'Rust: frozenset[int] を set[int] へ代入する形を静的に止める（タスク 3-1 で新設）。py: frozenset を知らず NameError'
    'function_attrs'                 ='Rust: 関数の値の __name__ / __qualname__ / __doc__（タスク 1-14 で新設）。py: 例題を構文解析できず ParseError（expected RPAREN, got STR）'
    'function_attrs_error'           ='Rust: __doc__ は Option[str] なので str へ束縛できない（タスク 1-14 で新設）。py: 関数の値に __doc__ が無く AttributeError'
    'py_function_attrs'              ='Rust: Python の関数の __name__ / __qualname__ / __doc__ と入れ子の関数の束縛（タスク 1-14 で新設・CPython と同じ出力）。py: import[py] したモジュールの run を引けず AttributeError'
    'py_property_typed'              ='Rust: getter の戻り値の注釈が property の型になる（タスク 6-8 で新設）。py: import[py] したモジュールのクラスを引けず AttributeError'
    'py_property_typed_error'        ='Rust: float の property を str の変数へ束縛する形を静的に止める（タスク 6-8 で新設）。py: import[py] したモジュールのクラスを引けず AttributeError'
    'py_cached_property'             ='Rust: functools.cached_property（タスク 6-5 で新設・CPython と同じ出力）。py: import[py] したモジュールの run を引けず AttributeError'
    'format_spec'                    ='Rust: 書式指定の小言語（format / f-string の {x:spec} / str.format・タスク 3-4 で新設・CPython と同じ出力）。py: 例題を構文解析できず ParseError（expected RPAREN, got STR）'
    'format_spec_error'              ='Rust: format は str を返すので int の変数へ束縛できない（タスク 3-4 で新設）。py: 型検査をせず実行して    42 を出す'
    'py_format'                      ='Rust: Python の f-string の書式指定・str.format・__format__（タスク 3-4 で新設・CPython と同じ出力）。py: import[py] したモジュールの run を引けず AttributeError'
    'py_varargs_tuple'               ='Rust: Python の関数の *args はタプル（タスク 1-16 で新設・CPython と同じ出力）。py: import[py] したモジュールの run を引けず AttributeError'
    'file_lines'                     ='Rust: どのファイルにも readline / readlines / writelines / close() / for line in f（タスク 5-4 で新設）。py: ファイルに writelines が無く AttributeError'
    'file_lines_error'               ='Rust: Python の形の閉じたファイルを回すと CPython と同じ ValueError（タスク 5-4 で新設）。py: ファイルを回せず TypeError'
    'py_file_methods'                ='Rust: Python のファイルのメソッドと with の終わりで閉じる（タスク 5-4 で新設・CPython と同じ出力）。py: import[py] したモジュールの run を引けず AttributeError'
    'str_char_positions'             ='Rust: 文字列の長さと位置は文字の単位（len / find / startswith の start・end / zfill / expandtabs・タスク 1-17 で新設・CPython と同じ出力）。py: str に rfind が無く AttributeError'
    'str_char_positions_error'       ='Rust: index で見つからなければ CPython と同じ ValueError（タスク 1-17 で新設）。py: str に index が無く AttributeError'
    'eof_error'                      ='Rust: Arrow のファイルの EOFError / BOFError を捕まえられる（タスク 4-5 で新設）。py: read_line が終わりで誤りを出さず while True が終わらない'
    'lazy_iterators'                 ='Rust: enumerate / zip が遅延のジェネレーター・next(it, default)・EndOfIteration を except StopIteration で捕まえる（タスク 3-5 で新設・CPython と同じ出力）。py: enumerate が元を先に全部取り出す'
    'py_object'                      ='Rust: 組み込みのクラス object と番兵（タスク 6-10 で新設・po.run() は CPython と同じ出力）。py: `===` の後の f-string を読めず ParseError'
    'py_object_error'                ='Rust: Arrow のソースの object() は静的な誤り（タスク 6-10 で新設）。py: object が無く実行時の NameError'
    'str_falls_back_to_repr'         ='Rust: str / print / f-string が __str__ の無いインスタンスで __repr__ を呼び、コンテナの要素は repr で出す（タスク 1-15 で新設・CPython と同じ出力）。py: 例題を構文解析できず ParseError（expected RPAREN, got STR）'
    'str_repr'                       ='Rust: 文字列の repr が CPython の unicode_repr と同じ（"it''s" / \n / \x00 の形・タスク 1-13 で新設）。py: コンテナの中の文字列を引用符なしで出す'
    'py_property_error'              ='Rust: setter の無い property への代入が CPython と同じ AttributeError（タスク 6-1 で新設）。py: import[py] したモジュールの run を引けず AttributeError'
    'py_exception_hierarchy'         = 'Rust: 組み込みの例外を継承した Python のクラスを作れ、多段の継承でも except が効く（10-19 で新設）。py: import[py] したモジュールの run を引けず AttributeError'
    'metafn_template'                = 'py: メタ関数（exprconst / code: / quote / ^）を字句解析できないので ParseError（2-8 で新設。Rust 側は展開時に単相化した Pair[str, int] のメタ情報を引く）'
    'metafn_module'                  = 'py: メタ関数（exprconst / code: / quote）を字句解析できないので ParseError（2-12 で新設。Rust 側は import 先のモジュールをモジュールごとに展開して実行する）'
    'metafn'                         = 'py: メタ関数（exprconst / code: / quote / <! !> / !装飾子）を字句解析できないので ParseError（2-2 で新設。Rust 側は展開して実行する）'
    'recursive_field_error'          = 'py: 値としての自己参照フィールドを検査しないので素通りして unreachable を出す（0-6 で新設）'
    'alias_newtype_generics'         = 'py: alias / new_type の右辺にジェネリクスを書けない（alias はカンマで ParseError、new_type は型名をスコープから引くので NameError・0-3 で新設）'
    'alias_newtype_generics_error'   = 'py: new_type の右辺の型名が実在するか検査しないので素通りして unreachable を出す（0-3 で新設）'
    'parse_ar'                       = 'py: 組み込み parse_ar 未実装（AST を値として返す・#56 で新設）'
    'parse_ar_error'                 = 'py: 組み込み parse_ar 未実装（#56 で新設）'
    'unregistered_type_call_error'   = 'Rust: 未登録の名前 byte の呼び出しは静的に name ''byte'' is not defined（何も出さない・#56 で新設、1-7 で tuple から byte へ）。py: 検査せず before を出してから実行時の NameError'
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
    'const_member_assign_error'      = 'Rust: const なメンバー（クラスの const・enum のメンバー）への代入をどの経路でも StaticTypeError（enum_member_type_plan.md 5-2 で新設）。py: 静的に検査せず実行して Counter.LIMIT = 5 の AttributeError で止まる'
    'protocol_in_container'          = 'py: protocol 型の束縛・メンバー解決が未実装（NameError/AttributeError になる・タスク 1.3 で新設）'
    'protocol_in_container_error'    = 'py: 容器の内側の protocol 適合検査が無い（そのまま実行してしまう・タスク 1.3 で新設）'
    'template_result_type_error'     = 'py: テンプレート実体化の結果型が無い（実行時の演算エラーになる・タスク 2.1 で新設）'
    'function_value_type_error'      = 'py: 関数値の型と分散規則の検査が無い（そのまま実行してしまう・タスク 2.2 で新設）'
    'enum_member_type'               = 'py: const クラス変数のクラス名経由アクセスが未対応（AttributeError になる・タスク 2.3 で新設）'
    'enum_member_type_error'         = 'py: enum .value の静的型が無い（int を str 変数へ入れて実行してしまう・タスク 2.3 で新設）'
    'enum_member_is_enum_type_error' = 'Rust: 別の enum との比較・代入、インスタンスからのメンバー（m.PLANE）・型の値からの value・廃止した enum_item_Color を StaticTypeError（enum_member_type_plan.md 4-1 で新設）。py: 静的に検査せず実行して m.PLANE の AttributeError で止まる'
    'and_or_result_type_error'       = 'py: and/or の結果型検査が無い（bool 変数に int を入れて実行してしまう・タスク 2.5 で新設）'
    'collection_elem_synthesis_error' = 'py: 混在リテラルの要素型合成が無い（list[int] へ混在を入れて実行してしまう・タスク 2.7 で新設）'
    'except_bind_and_match_expr'     = 'py: raise した例外への line 焼き込みが未実装（0 を返す・raise_span_fields と同根・タスク 2.8 で新設）'
    'except_bind_and_match_expr_error' = 'py: except as の束縛型と組み込み例外のフィールド型が無い（str を int 変数へ入れて実行してしまう・タスク 2.8 で新設）'
    'validity_checks'                = 'py: テンプレート型変数に対する is ガードが未対応（0 を返す・タスク 3.4 で新設）'
    'validity_checks_error'          = 'py: 型ガードの型名の存在検査が無い（死んだ腕のまま実行してしまう・タスク 3.4 で新設）'
    'validity_checks_enum_error'     = 'py: enum バリアント値の静的検査が無い（未呼出関数内を見逃す・タスク 3.4 で新設）'
    'upcast_only_error'              = 'py: 容器の適合方向を検査しないので素通りして実行してしまう（タスク 4.1 で新設・8.1 で list[Any] → list[int] へ書き換え）'
    'arith_operand_check'            = 'py: str % int（書式化）が未実装（TypeError になる・タスク 4.4 で新設）'
    'arith_operand_check_error'      = 'py: 算術の被演算子の静的検査が無い（実行時の TypeError になる・タスク 4.4 で新設）'
    'user_cast_site_error'           = 'py: __cast__ の受理地点を絞る検査が無い（int 変数に Conv が居座ったまま実行してしまう・タスク 4.3 で新設）'
    'compound_assign_two_stage'      = 'py: str %= int（書式化）が未実装（TypeError になる・arith_operand_check と同じ理由・タスク 5.1 で新設）'
    'compound_assign_two_stage_error' = 'py: 複合代入の結果型の検査が無い（Vec 変数に str が居座ったまま実行してしまう・タスク 5.1 で新設）'
    'collection_method_args_error'   = 'py: 可変長引数の要素型検査が無い（型が混ざったまま実行してしまう・タスク 5.2c で新設）'
    'new_type_ctor_arg'              = 'py: new_type の連鎖（Kg: Meters: float）が未実装で Kg(None) になる・タスク 5.6 で新設'
    'runtime_type_predicates'        = 'py: 実行時型判定の統一（uint と int の相互許容）が未実装で x is int が False になる・タスク 6.1 で新設'
    'overload_arg_types'             = 'py: オーバーロードの実行時ディスパッチが型を見ない（最初の定義を呼ぶので > int:hi になる）・タスク 5.7 で新設'
    'overload_arg_types_error'       = 'py: オーバーロードの実引数型による解決が無い（黙って実行してしまう・タスク 5.7 で新設）'
    'member_existence'               = 'py: protocol のフィールド要求を method として誤判定する（type Impl does not satisfy protocol HasN・impl_python 側の既知の限界）'
    'cross_type_equality'            = 'py: Option[T] / is None の扱いが未実装（タスク 7.6 で新設）'
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
    'py_kwonly_error'                = 'py: 同上。impl_python は変換時に止めず属性欠落で落ちる（A6 で新設）'
    'py_mutarg'                      = 'py: python_converter が Rust 専用（INF-D で新設）'
    'py_mutarg_error'                = 'py: 同上。impl_python は可変性検査まで到達しない（INF-D で新設）'
    'py_stmt_errors_error'           = 'py: python_converter が Rust 専用。impl_python は未対応の節を捨てて素通しする（C1 で新設）'
    'py_for_unpack'                  = 'py: python_converter が Rust 専用（C5 で新設）'
    'py_for_unpack_error'            = 'py: 同上。impl_python は for ターゲットを素通しする（C5 で新設）'
    'py_literal_unpack'              = 'py: python_converter が Rust 専用（C6 で新設）'
    'py_literal_unpack_error'        = 'py: 同上。impl_python は `*` 展開を素通しする（C6 で新設）'
    'py_generator'                   = 'py: python_converter が Rust 専用（C3 で新設）'
    'py_generator_error'             = 'py: 同上。impl_python は yield from 等を素通しする（C3 で新設）'
    'py_match_error'                 = 'py: python_converter が Rust 専用。impl_python は match のパターンを素通しする（C2 で新設）'
    'py_type_alias'                  = 'py: python_converter が Rust 専用（C4 で新設）'
    'py_type_alias_error'            = 'py: 同上。impl_python は type 文を素通しする（C4 で新設）'
    'py_multi_assign'                = 'py: python_converter が Rust 専用（D で新設）'
    'py_multi_assign_error'          = 'py: 同上。impl_python は入れ子アンパックを素通しする（D で新設）'
    'py_walrus'                      = 'py: python_converter が Rust 専用（D4 で新設）'
    'py_walrus_error'                = 'py: 同上。impl_python は walrus の位置を検査しない（D4 で新設）'
    'py_lambda_error'                = 'py: python_converter が Rust 専用。impl_python は lambda の持ち上げ位置を検査しない（D5 で新設）'
    'py_int_stub_error'              = 'py: 同梱 .pyi スタブは Rust 専用。impl_python は型検査まで到達しない（E で新設）'
    'py_with'                        = 'py: python_converter が Rust 専用（H1 で新設）'
    'py_with_error'                  = 'py: 同上。impl_python は with のコンテキストマネージャを検査しない（H1 で新設）'
    'dict_spread'                    = 'py: impl_python は辞書リテラルの `**` 展開を解釈しない（Arrow 側の新構文・U3 で新設）'
    'py_dict_spread'                 = 'py: python_converter が Rust 専用（U3 で新設）'
    'call_spread'                    = 'py: impl_python は呼び出しの `*`/`**` 展開を解釈しない（Arrow 側の新構文・U4/U5 で新設）'
    'seq_spread'                     = 'py: impl_python は列リテラルの `*` 展開を解釈しない（Arrow 側の新構文・U3 で新設）'
    'comprehension_multi_target'     = 'py: impl_python は内包表記の多ターゲットを解釈しない（Arrow 側の新構文で新設）'
    'dict_iter_and_builtin_types'    = 'py: impl_python は dict の直接反復と enumerate/zip の型を持たない（新設）'
    'dict_iter_error'                = 'py: 同上（静的エラーまで到達しない）'
    'py_import_stdlib_error'         = 'py: python_converter が Rust 専用。stdlib import を明示エラーにせず素通しする（項目27 で新設）'
    'py_comprehension_multi'         = 'py: python_converter が Rust 専用（多ターゲット内包表記で新設）'
    'py_call_spread'                 = 'py: python_converter が Rust 専用（U4/U5 で新設）'
    'py_decorator_forward'           = 'py: python_converter が Rust 専用（A7 / フェーズ G で新設）'
    'py_defaults'                    = 'py: python_converter が Rust 専用（項目1 で新設）'
    'py_defaults_error'              = 'Rust: 変換できないデフォルト式（ジェネレータ式）が変換時の誤り（項目1 で新設・タスク 3-4 で踏む形を書式指定から入れ替え）。py: 変換時エラーを出さず素通しする'
    'py_reassign'                    = 'py: python_converter が Rust 専用（項目2 で新設）'
    'py_subscript'                   = 'py: python_converter が Rust 専用（項目3 で新設）'
    'py_slice'                       = 'py: python_converter が Rust 専用（項目4 で新設）'
    'py_membership'                  = 'py: python_converter が Rust 専用（項目12 で新設）'
    'py_identity'                    = 'py: python_converter が Rust 専用（項目13 で新設）'
    'py_tuple'                       = 'py: python_converter が Rust 専用（項目18 で新設）'
    'py_classvar'                    = 'py: python_converter が Rust 専用（項目5 で新設）'
    'py_inherit'                     = 'py: python_converter が Rust 専用（クラス継承サポートで新設）'
    'py_inherit_error'               = 'py: 同上。変換時エラーを出さず素通しする'
    'py_fstring'                     = 'py: python_converter が Rust 専用（項目19 で新設）'
    'py_comprehension'               = 'py: python_converter が Rust 専用（項目17 で新設）'
    'py_comprehension_error'         = 'py: 同上。変換時エラーを出さず素通しする（項目17 で新設）'
    'py_ellipsis'                    = 'py: python_converter が Rust 専用（項目21 で新設）'
    'py_varargs'                     = 'py: python_converter が Rust 専用（項目6・7 で新設）'
    'py_chained_compare'             = 'py: python_converter が Rust 専用（項目16 で新設）'
    'stale_arc_check'                = 'py: .arc を UTF-8 として読んで UnicodeDecodeError'
    'swd_nested_runner'              = 'py: バイナリを UTF-8 として読んで UnicodeDecodeError'
    'typed_abi'                      = 'py: バイナリを UTF-8 として読んで UnicodeDecodeError'
    # (c) 値の表示形式（repr）の違い
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
    'list_concat_repeat'             = 'py 古い: tuple + tuple が未対応（実測 RuntimeError）'
    'list_concat_repeat_error'       = 'py: 実行時エラーの出力形式が違う（Rust は色付きトレースバック・py は 1 行）'
    # ⚠ B6（モジュール本体からの自己呼び出し）で新設。**実測して確認済み**。
    #    py は自己呼び出し自体は正しく動く（差分は repr だけ）＝ Rust の修正が参照実装と一致した証拠。
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
    # ⚠ タスク 8.1（素の容器型注釈の禁止・案 A）の移行で新しく食い違った。**実測して確認済み**。
    #   どちらも以前は「実行時のフィールド型検査」を見せる例題だったが、素の `list` が
    #   書けなくなり `list[Any]` になったので **Rust は静的に停止**するようになった。
    #   py は静的検査を持たないので 1 行目から出力が出る。
    'field_class_type_error'         = 'py: フィールド代入の静的型検査が無く素通りする（Rust は 8.1 以降 StaticTypeError で停止し何も出力しない）'
    'field_type_runtime_error'       = 'py: 同上（Rust は 8.1 以降 StaticTypeError で停止し何も出力しない）'
    # ⚠ タスク 9.9（テンプレート trait の型引数）で新設。**実測して確認済み**。
    'trait_template_args_error'      = 'py: テンプレート trait の型引数を照合しないので素通りして「ここには到達しない」まで出る（Rust は StaticTypeError で停止・9.9）'
    # ⚠ タスク 8.5（注釈の型名が実在するか）で新設。**実測して確認済み**。
    'unknown_type_name_error'        = 'py: 型注釈の型名が実在するか検査しないので素通りして [] を出す（Rust は StaticTypeError で停止・8.5）'
    # ⚠ タスク 9.7（型引数を捨てない）で新設。**実測して確認済み**。
    'type_args_not_dropped_error'    = 'py: 型注釈の `[...]` を検査しないので素通りして「ここには到達しない」まで出る（Rust は ParseError で停止・9.7）'
    # (e) 実行時エラーの出力形式（Rust は色付きトレースバック・py は 1 行）
    'runtime_error'                  = 'py: 実行時エラーの出力形式が違う'
    'traceback_frame_names'          = 'py: トレースバックの形式が違う'
    'try_except'                     = 'py: 例外メッセージの形式が違う'
    'try_except_errors'              = 'py: 例外メッセージの形式が違う'
}

# ⚠ `archived/` と `practical_examples/` は意図的に対象外（タスク 9.3）。
#   理由と現状は scan_examples.ps1 の `$categoryDirs` のコメント、および
#   examples/archived/README.md ・ examples/practical_examples/README.md。
$categoryDirs = @('basics', 'collections', 'classes', 'typing', 'exceptions', 'async', 'apps', 'interop', 'repl')

$examples = $categoryDirs | ForEach-Object {
    Get-ChildItem (Join-Path $repo "examples/$_/*.ar") -ErrorAction SilentlyContinue
} | Where-Object {
    $n = $_.BaseName
    (-not ($skip -contains $n)) -and ($Filter -eq '' -or $n -like "*$Filter*")
} | Sort-Object FullName

# 1 プロセスを制限時間つきで走らせて stdout を返す（$null = タイムアウト）。
# ⚠⚠ 出力は**パイプではなくファイル**へ落とす（skill `vm-pitfalls` §4・compare_outputs.ps1 と同じ）。
#    js-proc / cs-proc の例題は子プロセス（node など）を起こし、それが arrow.exe より長生きしてパイプの書き込み端を
#    握る。以前はパイプを `ReadToEndAsync` で読んでいたので、arrow.exe が終わっても `.Result` が**永久に返らず**、
#    ゲート全体が 7 時間以上止まった（2026-10-13・ffi_value_exchange_plan.md 2-3 の例題で発覚）。
# ⚠ 読み方は以前と同じコンソールのエンコーディング（パイプの既定）。変えると非 ASCII の出力の比較が変わる。
#    書き込み側（長生きした子）がファイルを開いたままでも読めるよう、共有モードは ReadWrite。
$tmpDir = Join-Path $env:TEMP "compare_python_impl_$PID"
New-Item -ItemType Directory -Force $tmpDir | Out-Null
function Read-Shared([string]$path) {
    if (-not (Test-Path $path)) { return '' }
    $fs = [IO.File]::Open($path, 'Open', 'Read', 'ReadWrite')
    try { return (New-Object IO.StreamReader($fs, [Console]::OutputEncoding)).ReadToEnd() } finally { $fs.Dispose() }
}
function Invoke-Impl {
    param([string]$File, [string]$Arguments, [int]$Limit)
    $o = Join-Path $tmpDir ([Guid]::NewGuid().ToString() + '.out')
    $e = Join-Path $tmpDir ([Guid]::NewGuid().ToString() + '.err')
    $p = Start-Process -FilePath $File -ArgumentList $Arguments -WorkingDirectory $repo -NoNewWindow -PassThru `
        -RedirectStandardOutput $o -RedirectStandardError $e
    if (-not $p.WaitForExit($Limit * 1000)) {
        # 子プロセスごと止める（`Kill()` は arrow.exe だけで、node などが残る）
        & taskkill.exe /T /F /PID $p.Id 2>&1 | Out-Null
        return $null
    }
    return (Read-Shared $o)
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
Remove-Item -Recurse -Force $tmpDir -ErrorAction SilentlyContinue
if ($diff -eq 0 -and $timeout -eq 0) {
    Write-Host 'PYTHON-DIFF: clean' -ForegroundColor Green
    exit 0
} else {
    Write-Host 'PYTHON-DIFF: FAILED' -ForegroundColor Red
    exit 1
}
