// type_check/names.rs — 名前の存在の判定材料（フェーズ10 10-12）。
//
// 型検査は、どのスコープにも無い名前（`print(nonexistent(2))`）を静的に弾く。以前は `Unresolved` に
// 倒して素通しにしており、実行時の `NameError` まで分からなかった。
//
// ⚠⚠ 誤って「未定義」と言わないために、判定は保守的にしてある:
//   - 今のスコープで見えなくても、**プログラムのどこかで束縛される名前**は通す（`declared_anywhere`）。
//     前方参照（後で宣言される最上位の名前を関数の中から読む）や、スコープの細部の食い違いで
//     偽の誤りを出さないため。取りこぼすのは「同じ名前がどこか別の場所で束縛されている」誤りだけ。
//   - 実行時が宣言なしで解決する名前（組み込みの型・例外・関数）は [`RUNTIME_BUILTIN_NAMES`] で通す。
//   - import 先を読めていない環境（エディタ）では判定しない（`registry_incomplete`）。

use std::collections::HashSet;

use crate::ast::{Expr, Stmt};

/// 実行時が**宣言なしで**解決する名前（組み込みの大域・組み込み関数）。
///
/// ⚠⚠ **実行時の表の写し**。出どころは 2 つ:
///   - `Interpreter::builtin_global_scope`（`register_builtin_globals` の型・例外・`path` など、`Signal`・`EventLoop`）
///   - 組み込み関数の表（`eval/builtins.rs` の `eval_builtin_evaled` / `eval_builtin_ident_call` の腕・`VM_BUILTIN_NAMES`）
///   ずれると「実行時は動くのに静的に未定義」という偽の誤りになるので、`interpreter/tests` の
///   `checker_knows_every_runtime_builtin_name` が**両方向**を突き合わせている。表を変えたら両方を直すこと。
/// 実行時の組み込みの大域（`Interpreter::builtin_global_scope`）が占める名前（ffi_value_exchange_plan.md 7-25）。
///
/// 組み込みの型名（と `len` / `id` / `Ok` / `Err` / `begin` / `last` / `EventLoop`）は、どのスレッドでも大域で見える名前で、
/// 同じ名前の変数は宣言できない（`VariableRedeclaration`・利用者の決定）。⚠ 以前は型検査の大域に無いもの（`Signal` /
/// `Index` / `pointer` / `EventLoop` / `path` / `Size` …）だけ型検査を通り、実行時に `NameError: variable '…' is already
/// declared` になっていた。組み込みの**関数**（`print` / `repr` …）は大域を占めないので、ここには無い（隠せる）。
/// ⚠ 実行時の表と**両方向**で一致することを `interpreter/tests` の `checker_knows_the_runtime_global_names` が見る。
pub(crate) const RUNTIME_GLOBAL_NAMES: &[&str] = &[
    "int", "uint", "str", "bytes", "float", "complex", "bool", "dict", "set", "function", "len", "slice",
    "pointer", "id", "Error",
    // 組み込みの例外クラスも大域を占める（表は [`BUILTIN_EXCEPTIONS`]・[`is_runtime_global_name`] が足す）。
    "Path", "Size", "Index", "begin", "last", "FileOpenMode", "StartPoint", "ByteRecognizingMode",
    "Encoding", "AsyncManager", "Async", "Signal", "EventLoop", "Ok", "Err",
];

/// 実行時の組み込みの大域が占める名前か（[`RUNTIME_GLOBAL_NAMES`]）。
pub(crate) fn is_runtime_global_name(name: &str) -> bool {
    RUNTIME_GLOBAL_NAMES.contains(&name) || is_builtin_exception(name)
}

pub(crate) const RUNTIME_BUILTIN_NAMES: &[&str] = &[
    // 組み込みの型（`register_builtin_globals`）
    "int", "uint", "str", "bytes", "float", "complex", "bool", "dict", "set", "function", "len", "slice",
    "pointer", "id", "Error",
    // 組み込みの例外クラスは [`BUILTIN_EXCEPTIONS`]（実行時も同じ表から登録する）。
    // 組み込みのクラス・列挙・名前空間
    "Path", "Size", "Index", "begin", "last", "FileOpenMode", "StartPoint", "ByteRecognizingMode",
    "Encoding", "AsyncManager", "Async", "Signal", "EventLoop",
    // Result の構築子（`Ok(v)` / `Err(e)`）
    "Ok", "Err",
    // 組み込み関数（`eval/builtins.rs`）
    "print", "range", "next", "repr", "enumerate", "zip", "getenv", "open", "close",
    "create_flat_int_list", "flat_get_int", "flat_set_int", "parse_ar",
    // 値として・呼んで作る型（`list(it)` / `tuple(it)`・python_builtins_plan.md のタスク 1-7 / `frozenset(it)`・3-1）。
    "list", "tuple", "frozenset", "bytes",
    // Python の組み込み（python_builtins_plan.md のフェーズ 2〜・本体は `eval/py_builtins.rs`）。
    "isinstance", "type", "getattr", "hasattr", "setattr", "issubclass", "callable",
    "all", "any", "min", "max", "sum", "sorted", "reversed", "abs", "round", "divmod", "pow",
    "map", "filter", "iter", "hash", "ord", "chr", "hex", "oct", "bin", "ascii",
    // 書式指定の小言語（タスク 3-4・f-string の `{x:spec}` の脱糖先）
    "format",
    // 変換器が Python の `with` の終わりに置く（ファイルを閉じる・タスク 5-4）
    "__py_with_exit",
    // Arrow で書いた組み込みのクラス（`eval/py_builtins.rs` の `PY_BUILTIN_CLASS_NAMES`・タスク 6-1 / 6-5）
    "property", "cached_property",
];

/// 実行時が宣言なしで解決する名前か。
pub(crate) fn is_runtime_builtin_name(name: &str) -> bool {
    RUNTIME_BUILTIN_NAMES.contains(&name) || is_builtin_exception(name)
}

/// 組み込みの例外・警告クラスと、その親（CPython 3.12 の階層・python_builtins_plan.md のタスク 4-1）。
///
/// **唯一の表**。実行時のクラスの登録（`built_in_types.rs` の `register_builtin_globals`）・
/// 内部のエラーの文字列を例外へ変える照合（`exceptions.rs` の `make_internal_raised_error`）・
/// Python のクラスが継承したときのフィールドの並び（`py_class_field_order`）・型検査のレジストリ
/// （`registry/builder.rs`）と大域の束縛（`TypeChecker::new`）が、みなここを読む
/// （以前は 3 つの表を手で揃えていた）。
///
/// - 親が `""` なのは根（`BaseException`）だけ。
/// - `AccessError` は Arrow 独自（アクセス制御の違反）。`BOFError` も Arrow 独自（ファイルを逆向きに読んで先頭に着いた）。
///   `FfiTypeError` / `ForeignError` も Arrow 独自（外部言語との境界・ffi_value_exchange_plan.md 7-4）。
/// - `IOError` / `EnvironmentError` / `WindowsError` は CPython では `OSError` **そのもの**（別名）。
///   ここでは `OSError` の子として載せ、互いに捕まえ合うのは [`builtin_exception_bases`] で作る。
///   ⚠ クラスは別のまま（`raise IOError("x")` の表示は `IOError: x`。CPython は `OSError: x`）。
/// - ⚠ 意味が要るもの（`SystemExit` / `KeyboardInterrupt` / `ImportError` / `UnicodeDecodeError` /
///   `ExceptionGroup` …）は載せていない（python_builtins_plan.md の 4.3）。
pub(crate) const BUILTIN_EXCEPTIONS: &[(&str, &str)] = &[
    ("BaseException", ""),
    ("GeneratorExit", "BaseException"),
    ("Exception", "BaseException"),
    ("ArithmeticError", "Exception"),
    ("FloatingPointError", "ArithmeticError"),
    ("OverflowError", "ArithmeticError"),
    ("ZeroDivisionError", "ArithmeticError"),
    ("AssertionError", "Exception"),
    ("AttributeError", "Exception"),
    ("BufferError", "Exception"),
    // Arrow のファイルの `read()` / `read_line()` / `read_letter()` が終わりで出す（`EOFError: EOF`・タスク 4-5）。
    // ⚠ 以前は表に無かったので、`except EOFError` は静的な誤り、`except Exception` も素の `except:` も捕まえなかった。
    ("EOFError", "Exception"),
    ("LookupError", "Exception"),
    ("IndexError", "LookupError"),
    ("KeyError", "LookupError"),
    ("NameError", "Exception"),
    ("UnboundLocalError", "NameError"),
    ("OSError", "Exception"),
    ("IOError", "OSError"),
    ("EnvironmentError", "OSError"),
    ("WindowsError", "OSError"),
    ("BlockingIOError", "OSError"),
    ("ChildProcessError", "OSError"),
    ("ConnectionError", "OSError"),
    ("BrokenPipeError", "ConnectionError"),
    ("ConnectionAbortedError", "ConnectionError"),
    ("ConnectionRefusedError", "ConnectionError"),
    ("ConnectionResetError", "ConnectionError"),
    ("FileExistsError", "OSError"),
    ("FileNotFoundError", "OSError"),
    ("InterruptedError", "OSError"),
    ("IsADirectoryError", "OSError"),
    ("NotADirectoryError", "OSError"),
    ("PermissionError", "OSError"),
    ("ProcessLookupError", "OSError"),
    ("TimeoutError", "OSError"),
    ("ReferenceError", "Exception"),
    ("RuntimeError", "Exception"),
    ("NotImplementedError", "RuntimeError"),
    ("RecursionError", "RuntimeError"),
    ("StopIteration", "Exception"),
    // Arrow 独自: 尽きたイテレータの `next()`（`EndOfIteration: generator is exhausted`・`for` が内部で受ける印）。
    // `StopIteration` の子なので `except StopIteration`（Python の書き方）でも `except EndOfIteration` でも捕まる
    // （python_builtins_plan.md のタスク 3-5。以前は表に無い名前で、どの `except` も素通りしてプログラムが止まった）。
    ("EndOfIteration", "StopIteration"),
    ("SyntaxError", "Exception"),
    ("IndentationError", "SyntaxError"),
    ("TabError", "IndentationError"),
    ("SystemError", "Exception"),
    ("TypeError", "Exception"),
    ("ValueError", "Exception"),
    ("UnicodeError", "ValueError"),
    ("UnicodeTranslateError", "UnicodeError"),
    // bytes の符号化・復号の失敗（`ops::codecs`・タスク 3-1）。⚠ CPython の 5 引数の構築子と `encoding` / `start` /
    //   `end` / `reason` の属性は無い（内部で起きたものは文言だけを持つ・4-2 の既知の差と同じ）。
    ("UnicodeDecodeError", "UnicodeError"),
    ("UnicodeEncodeError", "UnicodeError"),
    ("AccessError", "Exception"),
    // 外部言語との境界（ffi_value_exchange_plan.md 7-4・Arrow 独自）: 宣言と食い違う値（`TypeError` の派生なので
    // `except TypeError` でも捕まる）と、相手の言語で投げられた例外。
    ("FfiTypeError", "TypeError"),
    ("ForeignError", "Exception"),
    // Arrow 独自: ファイルを逆向きに読んで（`read(backward=True)` など）先頭に着いたとき（`BOFError: BOF`・タスク 4-5）。
    // `EOFError` の向きを逆にしたものだが、子にはしない（`except EOFError` が逆向きの終わりまで捕まえると読み違える）。
    ("BOFError", "Exception"),
    ("Warning", "Exception"),
    ("BytesWarning", "Warning"),
    ("DeprecationWarning", "Warning"),
    ("EncodingWarning", "Warning"),
    ("FutureWarning", "Warning"),
    ("ImportWarning", "Warning"),
    ("PendingDeprecationWarning", "Warning"),
    ("ResourceWarning", "Warning"),
    ("RuntimeWarning", "Warning"),
    ("SyntaxWarning", "Warning"),
    ("UnicodeWarning", "Warning"),
    ("UserWarning", "Warning"),
];

/// 組み込みの値の**メソッドの名前**（実行時の `eval_method_call_full` の腕・python_builtins_plan.md のタスク 1-10）。
///
/// 値として読める名前の表（`xs.append` / `s.upper` は受け手を束縛した関数になる）。実行時（束縛メソッドを
/// 作るか・`eval/attrs.rs`）と型検査（読みの存在検査・`infer_attr`）が同じ表を読む。種類の名前は実行時の
/// `runtime_type_name`（`list` / `fixed_list` / …）。
/// ⚠ 実行時のメソッドの腕を足したら**ここにも足す**。テスト `builtin_value_methods_are_dispatched` が
///   表の名前を実際に呼んで「無いメソッド」の誤りにならないことを確かめる（逆向きは見ない）。
/// ⚠ `complex` の `real` / `imag` / `angle` は載せない（CPython では値の属性。読むと値が返ると期待される）。
pub(crate) const BUILTIN_VALUE_METHODS: &[(&str, &[&str])] = &[
    (
        "str",
        &[
            "__iter__", "capitalize", "center", "chars", "contains", "count", "encode", "endswith",
            "expandtabs", "find", "findall", "format", "index", "isalnum", "isalpha", "isascii", "isdigit",
            "islower", "isnumeric", "isprintable", "isspace", "isupper", "join", "ljust", "lower", "lstrip",
            "match", "ord", "partition", "regex_split", "removeprefix", "removesuffix", "replace", "rfind",
            "rindex", "rjust", "rpartition", "rsplit", "rstrip", "search", "split", "splitlines", "startswith",
            "strip", "sub", "swapcase", "title", "upper", "zfill",
        ],
    ),
    ("list", &["__iter__", "append", "pop", "sort"]),
    ("dict", &["__iter__", "item", "items", "key", "keys", "values"]),
    (
        "set",
        &[
            "__iter__", "add", "clear", "copy", "difference", "discard", "intersection", "isdisjoint",
            "issubset", "issuperset", "pop", "remove", "symmetric_difference", "union",
        ],
    ),
    // 書き換えないメソッドだけ（`classes/set_methods.rs` の `SET_QUERY_METHODS`・タスク 3-1）。
    (
        "frozenset",
        &[
            "__iter__", "copy", "difference", "intersection", "isdisjoint", "issubset", "issuperset",
            "symmetric_difference", "union",
        ],
    ),
    // `classes/bytes_methods.rs`（タスク 3-1）。
    (
        "bytes",
        &[
            "__iter__", "count", "decode", "endswith", "find", "hex", "index", "isalnum", "isalpha", "isascii",
            "isdigit", "islower", "isspace", "isupper", "join", "lower", "lstrip", "replace", "rfind", "rindex",
            "rstrip", "split", "startswith", "strip", "upper",
        ],
    ),
    ("fixed_list", &["__contains__", "__iter__", "allocated_size", "append"]),
    ("generator", &["__iter__", "close", "next"]),
    // `classes/object_methods.rs` の `exec_file_method`（Arrow の名前と、Python の名前・タスク 5-4）。
    (
        "FileObject",
        &[
            "close", "flush", "read", "read_letter", "read_line", "readline", "readlines", "tell", "write",
            "write_line", "writelines",
        ],
    ),
];

/// `kind`（実行時の型の名前）の値が `name` というメソッドを持つか（[`BUILTIN_VALUE_METHODS`]）。
pub(crate) fn is_builtin_value_method(kind: &str, name: &str) -> bool {
    BUILTIN_VALUE_METHODS.iter().any(|(k, ms)| *k == kind && ms.contains(&name))
}

/// CPython で `OSError` と同じクラスを指す名前（[`BUILTIN_EXCEPTIONS`] の doc）。
const OSERROR_ALIASES: [&str; 3] = ["IOError", "EnvironmentError", "WindowsError"];

/// 組み込みの例外クラスの名前か。
pub(crate) fn is_builtin_exception(name: &str) -> bool {
    BUILTIN_EXCEPTIONS.iter().any(|(n, _)| *n == name)
}

/// 組み込みの例外クラスの基底の名前（`Error` trait と、親から根までの祖先を**平らに**並べたもの）。
///
/// 実行時の照合（`ClassValue::is_a`）は `bases` の**直接の名前だけ**を見て推移をたどらないので、
/// 祖先をすべて載せる（`except LookupError` が `KeyError` を捕まえる）。
///
/// `OSError` の別名（`IOError` …）の扱いは `mutual` で変わる:
/// - `true`（実行時）: `OSError` と別名は**互いを**基底に持ち、`OSError` の子孫は別名をすべて持つ
///   （`except IOError` が `OSError` も `FileNotFoundError` も捕まえる・CPython と同じ）。
/// - `false`（型検査）: 別名は `OSError` の子にとどめる（`OSError` の子孫は別名を持つ）。
///   ⚠⚠ 型検査の基底をたどる関数（`collect_class_field_details` など）は**循環を見ない**ので、
///   互いを基底にすると止まらない。代わりに `let e: IOError = OSError(..)` が静的に通らない。
pub(crate) fn builtin_exception_bases(name: &str, mutual: bool) -> Vec<String> {
    let mut out = vec!["Error".to_string()];
    let parent_of = |n: &str| BUILTIN_EXCEPTIONS.iter().find(|(c, _)| *c == n).map(|(_, p)| *p);
    let mut cur = parent_of(name).unwrap_or("");
    while !cur.is_empty() {
        out.push(cur.to_string());
        cur = parent_of(cur).unwrap_or("");
    }
    let is_alias = OSERROR_ALIASES.contains(&name);
    let in_os_group = name == "OSError" || out.iter().any(|b| b == "OSError");
    if in_os_group && (mutual || !is_alias && name != "OSError") {
        out.extend(OSERROR_ALIASES.iter().filter(|a| **a != name).map(|a| a.to_string()));
    }
    out
}

/// プログラムの**どこかで束縛される名前**をすべて集める（保守的・10-12）。
///
/// ⚠ `import` の本体（別モジュール）へは降りない。モジュールの名前は修飾なしでは見えない
///   （名前空間の分離・2026-09-26）ので、`import m` の後の `helper(2)` は未定義として弾く。
pub(crate) fn collect_declared_anywhere(stmts: &[Stmt], out: &mut HashSet<String>) {
    for stmt in stmts {
        crate::decl_names::each_declared_name(stmt, &mut |name, _, _| {
            out.insert(name.to_string());
        });
        crate::stmt_walk::each_subpart(stmt, &mut |part| {
            use crate::stmt_walk::StmtPart as P;
            match part {
                P::Expr(e) | P::MatchPattern(e) => collect_declared_in_expr(e, out),
                P::Control(b) | P::GenBody(b) | P::TypeBody(b) | P::ProtocolBody(b) | P::AsyncBody(b) => {
                    collect_declared_anywhere(b, out)
                }
                P::FnBody { params, body } => {
                    for p in params {
                        out.insert(p.name.clone());
                    }
                    collect_declared_anywhere(body, out);
                }
                P::ForTarget(t) | P::ExceptAlias(t) => {
                    out.insert(t.to_string());
                }
                // 別モジュールの本体・既存の名前への代入（束縛ではない）。
                P::ModuleBody(_) | P::TargetName(_) => {}
            }
        });
    }
}

fn collect_declared_in_expr(e: &Expr, out: &mut HashSet<String>) {
    crate::expr_walk::each_subpart(e, &mut |part| {
        use crate::expr_walk::SubPart as P;
        match part {
            P::Plain(x) | P::Control(x) | P::MatchPattern(x) => collect_declared_in_expr(x, out),
            P::Body(b) => collect_declared_anywhere(b, out),
            P::ForTarget(t) => {
                out.insert(t.to_string());
            }
        }
    });
}


/// プログラムのどこかで import されるモジュール（`a.b`）を、親（`a`）の子として集める
/// （`TypeChecker::module_children`・CPython 準拠・2026-10-02）。
///
/// ⚠ import は最上位にしか書けない（10-16）ので、たどるのは最上位の文と、import したモジュールの本体だけ。
/// ⚠ 外部言語（cpp / cs / js / rs）のドット区切りはファイルの場所なので数えない。
pub(super) fn collect_module_children(
    stmts: &[Stmt],
    out: &mut std::collections::HashMap<String, std::collections::BTreeSet<String>>,
) {
    for st in stmts {
        if let Stmt::Import { lang, module, body, .. } | Stmt::FromImport { lang, module, body, .. } = st {
            if crate::module_path::has_packages(lang) {
                if let Some((last, parent)) = module.split_last() {
                    if !parent.is_empty() {
                        out.entry(parent.join(".")).or_default().insert(last.clone());
                    }
                }
            }
            collect_module_children(body, out);
        }
    }
}
