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
pub(crate) const RUNTIME_BUILTIN_NAMES: &[&str] = &[
    // 組み込みの型（`register_builtin_globals`）
    "int", "uint", "str", "float", "complex", "bool", "dict", "set", "function", "len", "slice",
    "pointer", "id", "Error",
    // 組み込みの例外クラスは [`BUILTIN_EXCEPTIONS`]（実行時も同じ表から登録する）。
    // 組み込みのクラス・列挙・名前空間
    "path", "Size", "Index", "begin", "last", "FileOpenMode", "StartPoint", "ByteRecognizingMode",
    "Encoding", "AsyncManager", "Async", "Signal", "EventLoop",
    // Result の構築子（`Ok(v)` / `Err(e)`）
    "Ok", "Err",
    // 組み込みの列挙の要素のクラス（実行時の大域に載っている内部名）
    "enum_item_ByteRecognizingMode", "enum_item_Encoding", "enum_item_FileOpenMode", "enum_item_StartPoint",
    // 組み込み関数（`eval/builtins.rs`）
    "print", "range", "next", "repr", "enumerate", "zip", "getenv", "open", "close",
    "create_flat_int_list", "flat_get_int", "flat_set_int", "parse_ar",
    // 値として・呼んで作る型（`list(it)` / `tuple(it)`・python_builtins_plan.md のタスク 1-7）。
    "list", "tuple",
    // Python の組み込み（python_builtins_plan.md のフェーズ 2〜・本体は `eval/py_builtins.rs`）。
    "isinstance", "type", "getattr", "hasattr", "setattr", "issubclass", "callable",
    "all", "any", "min", "max", "sum", "sorted", "reversed", "abs", "round", "divmod", "pow",
    "map", "filter", "iter", "hash", "ord", "chr", "hex", "oct", "bin", "ascii",
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
/// - `AccessError` は Arrow 独自（アクセス制御の違反）。
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
    ("SyntaxError", "Exception"),
    ("IndentationError", "SyntaxError"),
    ("TabError", "IndentationError"),
    ("SystemError", "Exception"),
    ("TypeError", "Exception"),
    ("ValueError", "Exception"),
    ("UnicodeError", "ValueError"),
    ("UnicodeTranslateError", "UnicodeError"),
    ("AccessError", "Exception"),
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
