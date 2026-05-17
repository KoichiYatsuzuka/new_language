/// test_lang インタープリタのエントリーポイント。
///
/// パイプラインは以下の順序で処理を行う:
///   ソースファイル → Lexer（字句解析）→ Parser（構文解析）→ TypeChecker（静的型検査）→ Interpreter（実行）
mod ast;
mod interpreter;
mod lexer;
mod parser;
mod partial_compiler;
mod python_converter;
mod token;
mod type_check;

use interpreter::{ExecResult, Interpreter};
use lexer::Lexer;
use parser::Parser;
use type_check::TypeChecker;

/// コマンドライン引数を解析して実行モードを返す。
///
/// モードの優先順位:
/// 1. `--compile <path>` / `-compile <path>` → `Mode::Compile`
/// 2. `-src <path>` → `Mode::Run`
/// 3. `-` で始まらない最初の引数 → `Mode::Run`
/// 4. 引数なし → `Mode::Stdin`
enum Mode {
    /// 通常実行モード: ソースファイルをパースして実行する。
    Run(String),
    /// コンパイルモード: `.tlc` (バイナリ) と `.tls` (スタブ) を生成する。
    Compile(String),
    /// 標準入力モード: stdin からソースを読み込んで実行する。
    Stdin,
}

fn parse_args() -> Mode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;

    while i < args.len() {
        match args[i].as_str() {
            "--compile" | "-compile" => {
                return args.get(i + 1).map(|p| Mode::Compile(p.clone())).unwrap_or_else(|| {
                    eprintln!("Error: --compile requires a file path");
                    std::process::exit(1);
                });
            }
            "-src" => {
                return args.get(i + 1).map(|p| Mode::Run(p.clone())).unwrap_or_else(|| {
                    eprintln!("Error: -src requires a file path");
                    std::process::exit(1);
                });
            }
            _ => {}
        }
        i += 1;
    }

    args.into_iter()
        .find(|a| !a.starts_with('-'))
        .map(Mode::Run)
        .unwrap_or(Mode::Stdin)
}

/// ソースコード文字列を受け取り、字句解析・構文解析・静的型検査・実行の全パイプラインを実行する。
///
/// # 引数
/// - `source`   : 実行するソースコード文字列（`.tl` ファイルの内容など）。
/// - `filename` : エラーメッセージのスパン情報に使用するファイル名（例: `"script.tl"` や `"<stdin>"`）。
///
/// # 戻り値
/// - `Ok(())`    : プログラムが正常終了した場合。
/// - `Err(msg)`  : パースエラー・静的型エラー・実行時エラーのいずれかが発生した場合。
///                 `msg` はユーザー向けのエラーメッセージ文字列。
///
/// # エラーの種類
/// - `ParseError`      : 構文解析に失敗した場合。
/// - `StaticTypeError` : 静的型検査で1件以上のエラーが検出された場合（全件を改行区切りで返す）。
/// - 実行時エラー      : インタープリタが `Raise` または内部エラー文字列を返した場合。
fn run_program(source: &str, filename: &str) -> Result<(), String> {
    // --- 字句解析: ソースをトークン列（Vec<Spanned>）に変換する ---
    let tokens = Lexer::new(source, filename).tokenize();

    // ソースファイルのディレクトリを解決する（import 時の検索基準）
    let source_dir = std::path::Path::new(filename)
        .parent()
        .map(|p| p.to_path_buf());

    // --- 構文解析: トークン列を AST（Vec<Stmt>）に変換する ---
    let stmts = Parser::new(tokens, source_dir.clone()).parse_program()
        .map_err(|e| format!("ParseError: {e}"))?;

    // --- 静的型検査: AST を走査してエラーを収集し、1件でもあれば全件報告して終了する ---
    let type_errors = TypeChecker::check(&stmts);
    if !type_errors.is_empty() {
        // 複数エラーを改行区切りで結合してまとめて返す
        let msg = type_errors.iter()
            .map(|e| e.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        return Err(msg);
    }

    // --- インタープリタの初期化とソーステキストの登録 ---
    // ソーステキストはエラー報告時のスタックトレース表示に使用される
    let mut interp = Interpreter::new();
    interp.add_source_text(filename, source);
    // ソースファイルのディレクトリを import 検索パスに追加する
    if let Some(dir) = &source_dir {
        interp.add_python_search_dir(dir.clone());
    }

    // --- 各トップレベル文を順番に実行する ---
    for stmt in &stmts {
        match interp.exec(stmt) {
            // `raise` 文が実行された場合: フォーマット済みエラーレポートを返す
            Ok(ExecResult::Raise(raised)) => {
                return Err(Interpreter::format_error_report(&raised));
            }
            // 正常終了: 次の文へ続く
            Ok(_) => {}
            // 内部シグナル `\x00__raise__`: インタープリタが例外を保持している場合に送出される
            Err(e) if e == "\x00__raise__" => {
                let msg = interp.take_current_exception()
                    .map(|r| Interpreter::format_error_report(&r))
                    .unwrap_or_else(|| "UnhandledException: (no details available)".to_string());
                return Err(msg);
            }
            // その他の実行時エラー文字列をそのまま返す
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

/// プログラムのエントリーポイント。
fn main() {
    match parse_args() {
        Mode::Run(path) => {
            // .tlc: extract embedded source first, then run normally
            let (source, filename) = if path.ends_with(".tlc") {
                match partial_compiler::load_tlc(std::path::Path::new(&path)) {
                    Ok((name, src)) => (src, format!("<compiled:{name}>")),
                    Err(e) => {
                        eprintln!("Error loading {path}: {e}");
                        std::process::exit(1);
                    }
                }
            } else {
                (read_file(&path), path)
            };
            if let Err(e) = run_program(&source, &filename) {
                eprintln!("{e}");
                std::process::exit(1);
            }
        }

        Mode::Stdin => {
            use std::io::Read;
            let mut buf = String::new();
            std::io::stdin().read_to_string(&mut buf).expect("failed to read stdin");
            if let Err(e) = run_program(&buf, "<stdin>") {
                eprintln!("{e}");
                std::process::exit(1);
            }
        }

        Mode::Compile(path) => {
            compile_module(&path);
        }
    }
}

/// ファイルを読み込む。失敗したら stderr に出力して終了する。
fn read_file(path: &str) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| {
        eprintln!("Error reading {path}: {e}");
        std::process::exit(1);
    })
}

/// `--compile` モード: ソースをパース・型検査して `.tlc` と `.tls` を生成する。
fn compile_module(path: &str) {
    let source = read_file(path);

    let tokens = Lexer::new(&source, path).tokenize();
    let source_dir = std::path::Path::new(path).parent().map(|p| p.to_path_buf());

    let stmts = Parser::new(tokens, source_dir).parse_program().unwrap_or_else(|e| {
        eprintln!("ParseError: {e}");
        std::process::exit(1);
    });

    let type_errors = TypeChecker::check(&stmts);
    if !type_errors.is_empty() {
        for e in &type_errors {
            eprintln!("{e}");
        }
        std::process::exit(1);
    }

    match partial_compiler::compile(&source, &stmts, std::path::Path::new(path)) {
        Ok((tlc, tls)) => {
            println!("Compiled : {}", tlc.display());
            println!("Stub     : {}", tls.display());
        }
        Err(e) => {
            eprintln!("Compile error: {e}");
            std::process::exit(1);
        }
    }
}
