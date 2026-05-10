/// test_lang インタープリタのエントリーポイント。
///
/// パイプラインは以下の順序で処理を行う:
///   ソースファイル → Lexer（字句解析）→ Parser（構文解析）→ TypeChecker（静的型検査）→ Interpreter（実行）
mod ast;
mod interpreter;
mod lexer;
mod parser;
mod python_converter;
mod token;
mod type_check;

use interpreter::{ExecResult, Interpreter};
use lexer::Lexer;
use parser::Parser;
use type_check::TypeChecker;

/// コマンドライン引数を解析し、実行対象のファイルパスを返す。
///
/// 引数の解釈ルール:
/// 1. `-src <path>` フラグが見つかればその次の値をファイルパスとして返す。
///    `-src` の直後に値がない場合はエラーメッセージを出力して終了する。
/// 2. `-src` が存在しない場合は、`-` で始まらない最初の引数を位置引数として使用する。
/// 3. どちらも存在しない場合は `None` を返す（標準入力から読み込む）。
///
/// # 戻り値
/// `Some(path)` — 解析されたファイルパス文字列。
/// `None`       — ファイルパスが指定されていない（標準入力モード）。
fn parse_args() -> Option<String> {
    // プログラム名（第0引数）を除いた引数リストを収集する
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;

    // `-src` フラグを先に探す
    while i < args.len() {
        if args[i] == "-src" {
            // `-src` の次の引数をファイルパスとして返す。なければエラー終了する
            return args.get(i + 1).cloned().or_else(|| {
                eprintln!("Error: -src requires a file path");
                std::process::exit(1);
            });
        }
        i += 1;
    }

    // `-src` がなければ `-` で始まらない最初の引数を位置引数として使用する
    args.into_iter().find(|a| !a.starts_with('-'))
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
    // ソースファイルのディレクトリを import[py-int] の検索パスに追加する
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
///
/// 動作:
/// 1. `parse_args()` でファイルパスを取得する。
///    - パスが得られた場合: そのファイルを読み込んでソースとする。
///    - パスが得られなかった場合: 標準入力からソースを読み込む（ファイル名は `"<stdin>"`）。
/// 2. `run_program()` でパイプライン全体を実行する。
/// 3. エラーが発生した場合は標準エラー出力にメッセージを表示し、終了コード 1 で終了する。
fn main() {
    // ファイルパスがあればファイルから、なければ標準入力からソースを読み込む
    let (source, filename) = if let Some(path) = parse_args() {
        // ファイル読み込みに失敗した場合はエラーメッセージを出力して終了する
        let content = std::fs::read_to_string(&path).unwrap_or_else(|e| {
            eprintln!("Error reading {path}: {e}");
            std::process::exit(1);
        });
        (content, path)
    } else {
        // 標準入力からすべてのバイトを読み込む
        use std::io::Read;
        let mut buf = String::new();
        std::io::stdin().read_to_string(&mut buf).expect("failed to read stdin");
        (buf, "<stdin>".to_string())
    };

    // パイプライン実行: エラーがあれば stderr に出力して終了コード 1 で終了する
    if let Err(e) = run_program(&source, &filename) {
        eprintln!("{e}");
        std::process::exit(1);
    }
}
