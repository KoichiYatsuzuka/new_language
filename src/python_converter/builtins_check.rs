// python_converter/builtins_check.rs — 未対応の組み込みの参照を変換時に誤りにする
// （python_builtins_plan.md のタスク 1-1）。
//
// Python のモジュールが Arrow の持たない組み込み（`format` / `vars` / `exit` …）を参照していると、
// 以前は**その行を実行したときに初めて** `NameError` になった。変換は通り、呼ばれない関数の中なら
// 気づかないまま動いていた。ここでは**モジュールを読む時点で**（＝実行の前に）まとめて知らせる。
//
// ⚠ 決定（2026-10-06）: 警告ではなく**変換の誤り**にする。その名前を 1 箇所でも含むモジュールは、
//   実行しない枝にあっても丸ごと読めない（代わりに、動くはずのないコードが黙って読み込まれることが無い）。
//
// ## 「未対応」の判定
// CPython 3.12 の組み込みの名前（[`CPYTHON_BUILTINS`]）のうち、Arrow の実行時が解決できない名前
// （`type_check::names::is_runtime_builtin_name` が偽）。組み込みを足すと**自動で外れる**（表を 2 つ持たない）。
//
// ## 誤りにしない形（保守的に倒す）
// - モジュールの**どこかで束縛される名前**（代入・`def` / `class`・仮引数・`import`・`for` / `with` / `except` の
//   束縛・`global` / `nonlocal`・`match` の捕捉）。スコープは解かない（隠していれば通す）。
// - 変換器が形で受ける位置: デコレータ（`@staticmethod` / `@classmethod`。`@property` は変換器が別に誤りにする）、
//   クラスの基底（`class C(object)`）、`super().m(..)` の `super`。
// - `from m import *` があるモジュールは判定しない（何が束縛されるか分からない）。

use std::collections::HashSet;
use std::convert::Infallible;

use rustpython_parser::ast as py;
use rustpython_parser::ast::fold::Fold;
use rustpython_parser::text_size::TextRange;

/// CPython 3.12 の `dir(builtins)` から、モジュールの属性（`__name__` / `__doc__` / `__package__` /
/// `__loader__` / `__spec__`）を除いた 153 名（python_builtins_plan.md の 1.1）。
const CPYTHON_BUILTINS: &[&str] = &[
    "ArithmeticError", "AssertionError", "AttributeError", "BaseException", "BaseExceptionGroup",
    "BlockingIOError", "BrokenPipeError", "BufferError", "BytesWarning", "ChildProcessError",
    "ConnectionAbortedError", "ConnectionError", "ConnectionRefusedError", "ConnectionResetError",
    "DeprecationWarning", "EOFError", "Ellipsis", "EncodingWarning", "EnvironmentError", "Exception",
    "ExceptionGroup", "False", "FileExistsError", "FileNotFoundError", "FloatingPointError", "FutureWarning",
    "GeneratorExit", "IOError", "ImportError", "ImportWarning", "IndentationError", "IndexError",
    "InterruptedError", "IsADirectoryError", "KeyError", "KeyboardInterrupt", "LookupError", "MemoryError",
    "ModuleNotFoundError", "NameError", "None", "NotADirectoryError", "NotImplemented",
    "NotImplementedError", "OSError", "OverflowError", "PendingDeprecationWarning", "PermissionError",
    "ProcessLookupError", "RecursionError", "ReferenceError", "ResourceWarning", "RuntimeError",
    "RuntimeWarning", "StopAsyncIteration", "StopIteration", "SyntaxError", "SyntaxWarning", "SystemError",
    "SystemExit", "TabError", "TimeoutError", "True", "TypeError", "UnboundLocalError", "UnicodeDecodeError",
    "UnicodeEncodeError", "UnicodeError", "UnicodeTranslateError", "UnicodeWarning", "UserWarning",
    "ValueError", "Warning", "WindowsError", "ZeroDivisionError", "__build_class__", "__debug__",
    "__import__", "abs", "aiter", "all", "anext", "any", "ascii", "bin", "bool", "breakpoint", "bytearray",
    "bytes", "callable", "chr", "classmethod", "compile", "complex", "copyright", "credits", "delattr",
    "dict", "dir", "divmod", "enumerate", "eval", "exec", "exit", "filter", "float", "format", "frozenset",
    "getattr", "globals", "hasattr", "hash", "help", "hex", "id", "input", "int", "isinstance", "issubclass",
    "iter", "len", "license", "list", "locals", "map", "max", "memoryview", "min", "next", "object", "oct",
    "open", "ord", "pow", "print", "property", "quit", "range", "repr", "reversed", "round", "set",
    "setattr", "slice", "sorted", "staticmethod", "str", "sum", "super", "tuple", "type", "vars", "zip",
];

/// CPython の組み込みのうち、Arrow の実行時が解決できない名前か。
///
/// ⚠ `True` / `False` / `None` は Python の構文木では定数で、名前としては現れない（ここに来ない）。
pub(crate) fn is_unsupported_builtin(name: &str) -> bool {
    CPYTHON_BUILTINS.contains(&name) && !crate::type_check::names::is_runtime_builtin_name(name)
}

/// モジュールが未対応の組み込みを参照していれば誤りにする（モジュールの冒頭の doc）。
///
/// 誤りの文言は名前ごとに最初の行を並べる（`'format' (line 3), 'exit' (line 9)`）。
pub(crate) fn check_unsupported_builtins(
    suite: &[py::Stmt],
    source: &str,
    filename: &str,
) -> Result<(), String> {
    let mut finder = BuiltinRefFinder::default();
    for stmt in suite.iter().cloned() {
        let Ok(_) = finder.fold_stmt(stmt);
    }
    if finder.star_import {
        return Ok(());
    }
    let mut reported: Vec<(String, usize)> = Vec::new();
    for (name, range) in &finder.loads {
        if finder.bound.contains(name)
            || finder.skipped.contains(&range.start())
            || !is_unsupported_builtin(name)
            || reported.iter().any(|(n, _)| n == name)
        {
            continue;
        }
        reported.push((name.clone(), line_of(source, usize::from(range.start()))));
    }
    if reported.is_empty() {
        return Ok(());
    }
    reported.sort_by_key(|(_, line)| *line);
    let list: Vec<String> = reported.iter().map(|(n, l)| format!("'{n}' (line {l})")).collect();
    Err(format!(
        "{filename}: Python builtin(s) not supported by Arrow: {} \
         (the module is rejected even if that code never runs; define the name in the module or avoid it)",
        list.join(", ")
    ))
}

/// バイト位置の行番号（1 始まり）。
fn line_of(source: &str, offset: usize) -> usize {
    source.as_bytes()[..offset.min(source.len())].iter().filter(|&&b| b == b'\n').count() + 1
}

/// 構文木を 1 周して、名前の読み・束縛・形で受ける位置を集める（`Fold` は中身を作り直さず通すだけ）。
#[derive(Default)]
struct BuiltinRefFinder {
    /// 読み（`ExprContext::Load`）の名前と位置。
    loads: Vec<(String, TextRange)>,
    /// モジュールのどこかで束縛される名前。
    bound: HashSet<String>,
    /// 変換器が形で受ける位置にある名前の開始位置（デコレータ・クラスの基底・`super()`）。
    skipped: HashSet<rustpython_parser::text_size::TextSize>,
    /// `from m import *` がある。
    star_import: bool,
}

impl BuiltinRefFinder {
    fn skip_name(&mut self, e: &py::Expr) {
        if let py::Expr::Name(n) = e {
            self.skipped.insert(n.range.start());
        }
    }

    /// デコレータの名前（`@staticmethod` / `@deco(...)` の `deco`）。
    fn skip_decorators(&mut self, decorators: &[py::Expr]) {
        for d in decorators {
            match d {
                py::Expr::Call(c) => self.skip_name(&c.func),
                other => self.skip_name(other),
            }
        }
    }
}

impl Fold<TextRange> for BuiltinRefFinder {
    type TargetU = TextRange;
    type Error = Infallible;
    type UserContext = ();

    fn will_map_user(&mut self, _user: &TextRange) -> Self::UserContext {}

    fn map_user(&mut self, user: TextRange, _context: Self::UserContext) -> Result<TextRange, Infallible> {
        Ok(user)
    }

    fn fold_stmt(&mut self, node: py::Stmt) -> Result<py::Stmt, Infallible> {
        match &node {
            py::Stmt::FunctionDef(f) => {
                self.bound.insert(f.name.to_string());
                self.skip_decorators(&f.decorator_list);
            }
            py::Stmt::AsyncFunctionDef(f) => {
                self.bound.insert(f.name.to_string());
                self.skip_decorators(&f.decorator_list);
            }
            py::Stmt::ClassDef(c) => {
                self.bound.insert(c.name.to_string());
                self.skip_decorators(&c.decorator_list);
                for b in &c.bases {
                    self.skip_name(b);
                }
            }
            py::Stmt::Global(g) => self.bound.extend(g.names.iter().map(|n| n.to_string())),
            py::Stmt::Nonlocal(g) => self.bound.extend(g.names.iter().map(|n| n.to_string())),
            py::Stmt::ImportFrom(i) if i.names.iter().any(|a| a.name.as_str() == "*") => {
                self.star_import = true;
            }
            _ => {}
        }
        py::fold::fold_stmt(self, node)
    }

    fn fold_expr(&mut self, node: py::Expr) -> Result<py::Expr, Infallible> {
        match &node {
            py::Expr::Name(n) => match n.ctx {
                py::ExprContext::Load => self.loads.push((n.id.to_string(), n.range)),
                _ => {
                    self.bound.insert(n.id.to_string());
                }
            },
            // `super().m(..)` の `super`（変換器が `<第 1 基底>.m(self, ..)` へ書き換える）。
            py::Expr::Call(c) if c.args.is_empty() && c.keywords.is_empty() => {
                if matches!(&*c.func, py::Expr::Name(n) if n.id.as_str() == "super") {
                    self.skip_name(&c.func);
                }
            }
            _ => {}
        }
        py::fold::fold_expr(self, node)
    }

    fn fold_arg(&mut self, node: py::Arg) -> Result<py::Arg, Infallible> {
        self.bound.insert(node.arg.to_string());
        py::fold::fold_arg(self, node)
    }

    fn fold_alias(&mut self, node: py::Alias) -> Result<py::Alias, Infallible> {
        let bound = match &node.asname {
            Some(a) => a.to_string(),
            None => node.name.as_str().split('.').next().unwrap_or("").to_string(),
        };
        self.bound.insert(bound);
        py::fold::fold_alias(self, node)
    }

    fn fold_excepthandler(&mut self, node: py::ExceptHandler) -> Result<py::ExceptHandler, Infallible> {
        let py::ExceptHandler::ExceptHandler(h) = &node;
        if let Some(n) = &h.name {
            self.bound.insert(n.to_string());
        }
        py::fold::fold_excepthandler(self, node)
    }

    fn fold_pattern(&mut self, node: py::Pattern) -> Result<py::Pattern, Infallible> {
        match &node {
            py::Pattern::MatchAs(p) => self.bound.extend(p.name.iter().map(|n| n.to_string())),
            py::Pattern::MatchStar(p) => self.bound.extend(p.name.iter().map(|n| n.to_string())),
            py::Pattern::MatchMapping(p) => self.bound.extend(p.rest.iter().map(|n| n.to_string())),
            _ => {}
        }
        py::fold::fold_pattern(self, node)
    }
}
