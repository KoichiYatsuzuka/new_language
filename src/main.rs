/// Arrow インタープリタのエントリーポイント。
///
/// パイプラインは以下の順序で処理を行う:
///   ソースファイル → Lexer（字句解析）→ Parser（構文解析）→ TypeChecker（静的型検査）→ Interpreter（実行）
mod ast;
#[cfg(test)]
mod frontend_tests;
mod interpreter;
mod lexer;
mod parser;
mod partial_compiler;
mod python_converter;
mod repl;
mod token;
mod type_check;
mod vm;

use interpreter::{ExecResult, Interpreter};
use lexer::Lexer;
use parser::Parser;
use type_check::TypeChecker;

/// インタープリタの実行モードを表す列挙型。
///
/// `parse_args()` が返す値として使われ、`main()` がモードに応じた処理へ分岐する。
enum Mode {
    /// 通常実行モード: ソースファイルをパースして実行する。第2要素はユーザー定義CLIパラメータ。
    Run(String, std::collections::HashMap<String, String>),
    /// コンパイルモード: `.arc` (バイナリ) と `.ars` (スタブ) を生成する。
    Compile(String),
    /// C# スタブ生成モード: .NET DLL から `.ars` スタブファイルを生成する。
    CompileCs(String),
    /// 標準入力モード: stdin からソースを読み込んで実行する。
    Stdin,
    /// REPL モード: stdin からブロックを受け取り、インタープリタを維持しながら実行する。
    Repl,
}

/// コマンドライン引数を解析して実行モードを返す。
///
/// モードの優先順位:
/// 1. `--compile <path>` / `-compile <path>` → `Mode::Compile`
/// 2. `-src <path>` → `Mode::Run`
/// 3. `--repl` → `Mode::Repl`
/// 4. `-` で始まらない最初の引数 → `Mode::Run`
/// 5. 引数なし → `Mode::Stdin`
///
/// `Mode::Run` には `--key value` 形式で渡されたユーザー定義パラメータも含まれる。
/// 値のないフラグ (`--flag`) は値 `"true"` として記録される。
fn parse_args() -> Mode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    let mut file_path: Option<String> = None;
    let mut cli_params: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();

    while i < args.len() {
        match args[i].as_str() {
            "--compile" | "-compile" => {
                return args
                    .get(i + 1)
                    .map(|p| Mode::Compile(p.clone()))
                    .unwrap_or_else(|| {
                        eprintln!("Error: --compile requires a file path");
                        std::process::exit(1);
                    });
            }
            "--compile-cs" | "-compile-cs" => {
                return args
                    .get(i + 1)
                    .map(|p| Mode::CompileCs(p.clone()))
                    .unwrap_or_else(|| {
                        eprintln!("Error: --compile-cs requires a .dll file path");
                        std::process::exit(1);
                    });
            }
            "-src" => {
                if let Some(p) = args.get(i + 1) {
                    file_path = Some(p.clone());
                    i += 2;
                    continue;
                } else {
                    eprintln!("Error: -src requires a file path");
                    std::process::exit(1);
                }
            }
            "--repl" => return Mode::Repl,
            // バイトコード VM モード: `--vm=off|auto|force` または `--vm off` 形式（Phase V）。
            arg if arg == "--vm" || arg.starts_with("--vm=") => {
                let mode_str = if let Some(eq) = arg.strip_prefix("--vm=") {
                    eq.to_string()
                } else {
                    let m = args.get(i + 1).cloned().unwrap_or_default();
                    i += 1;
                    m
                };
                cli_params.insert("__vm__".to_string(), mode_str);
                i += 1;
                continue;
            }
            arg if arg.starts_with("--") => {
                // User-defined parameter: --key [value]
                let key = arg[2..].to_string();
                let next = args.get(i + 1);
                // If the next token exists and does not itself look like a flag, treat it as the value.
                if let Some(val) = next.filter(|v| !v.starts_with("--")) {
                    cli_params.insert(key, val.clone());
                    i += 2;
                } else {
                    cli_params.insert(key, "true".to_string());
                    i += 1;
                }
                continue;
            }
            arg if !arg.starts_with('-') => {
                // Positional argument → file path (first one wins)
                if file_path.is_none() {
                    file_path = Some(arg.to_string());
                }
                i += 1;
                continue;
            }
            _ => {}
        }
        i += 1;
    }

    match file_path {
        Some(path) => Mode::Run(path, cli_params),
        None => Mode::Stdin,
    }
}

/// ANSI エスケープシーケンスを除去した文字列の表示幅（文字数）を返す。
fn ansi_display_len(s: &str) -> usize {
    let mut len = 0;
    let mut in_escape = false;
    for c in s.chars() {
        if in_escape {
            if c == 'm' {
                in_escape = false;
            }
        } else if c == '\x1b' {
            in_escape = true;
        } else {
            len += 1;
        }
    }
    len
}

/// ANSI コード付き文字列を視覚的幅 `width` に右パディングする。
fn ansi_pad(s: &str, width: usize) -> String {
    let visible = ansi_display_len(s);
    let pad = width.saturating_sub(visible);
    format!("{}{}", s, " ".repeat(pad))
}

/// ANSI コード付き文字列をスペース区切りで折り返す。各行の視覚的幅は `budget` 以内。
fn word_wrap_ansi(s: &str, budget: usize) -> Vec<String> {
    if budget < 4 {
        return vec![s.to_string()];
    }
    let mut lines: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut current_len = 0usize;
    for word in s.split(' ') {
        let wlen = ansi_display_len(word);
        if current.is_empty() {
            current.push_str(word);
            current_len = wlen;
        } else if current_len + 1 + wlen <= budget {
            current.push(' ');
            current.push_str(word);
            current_len += 1 + wlen;
        } else {
            lines.push(std::mem::take(&mut current));
            current.push_str(word);
            current_len = wlen;
        }
    }
    if !current.is_empty() || lines.is_empty() {
        lines.push(current);
    }
    lines
}

/// 現在のターミナル幅を返す。取得できない場合は 120 を返す。
fn terminal_width() -> usize {
    use terminal_size::{terminal_size, Width};
    terminal_size()
        .map(|(Width(w), _)| w as usize)
        .unwrap_or(120)
}

/// 静的型エラーをテーブル形式にフォーマットして返す。
fn format_static_errors(errors: &[type_check::StaticTypeError]) -> String {
    const R: &str = "\x1b[31m";
    const Y: &str = "\x1b[33m";
    const C: &str = "\x1b[1;36m";
    const X: &str = "\x1b[0m";

    let rows: Vec<(String, String, &str, String)> = errors
        .iter()
        .map(|e| {
            (
                e.file_str(),
                e.line_col_str(),
                e.error_type_str(),
                e.detail_str(),
            )
        })
        .collect();

    let w1 = rows
        .iter()
        .map(|(f, _, _, _)| f.len())
        .max()
        .unwrap_or(0)
        .max("File".len());
    let w2 = rows
        .iter()
        .map(|(_, l, _, _)| l.len())
        .max()
        .unwrap_or(0)
        .max("Line:Col".len());
    let w3 = "StaticTypeError".len().max("Error Type".len());

    // col separators: 3 × "  " = 6 chars; compute remaining width for message column
    let fixed = w1 + w2 + w3 + 6;
    let msg_budget = terminal_width().saturating_sub(fixed).max(20);

    let sep = format!(
        "{}{}{X}  {}{}{X}  {}{}{X}  {}{}{X}",
        Y,
        "─".repeat(w1),
        Y,
        "─".repeat(w2),
        Y,
        "─".repeat(w3),
        Y,
        "─".repeat(msg_budget)
    );
    let header = format!(
        "{}{}{}  {}{}{}  {}{}{}  {}{}{}",
        C,
        ansi_pad("File", w1),
        X,
        C,
        ansi_pad("Line:Col", w2),
        X,
        C,
        ansi_pad("Error Type", w3),
        X,
        C,
        "Message",
        X,
    );

    // blank prefix for continuation lines (cols 1-3 replaced by spaces)
    let blank_prefix = format!(
        "{}  {}  {}  ",
        " ".repeat(w1),
        " ".repeat(w2),
        " ".repeat(w3)
    );

    let mut lines = vec![header, sep];
    for (file, loc, etype, msg) in &rows {
        let msg_lines = word_wrap_ansi(msg, msg_budget);
        let first = msg_lines.first().map(String::as_str).unwrap_or("");
        lines.push(format!(
            "{}{}{}  {}{}{}  {}{}{}  {}",
            Y,
            ansi_pad(file, w1),
            X,
            Y,
            ansi_pad(loc, w2),
            X,
            R,
            ansi_pad(etype, w3),
            X,
            first,
        ));
        for cont in msg_lines.iter().skip(1) {
            lines.push(format!("{}{}", blank_prefix, cont));
        }
    }
    format!("\n\n{}\n\n", lines.join("\n"))
}

/// ソースコード文字列を受け取り、字句解析・構文解析・静的型検査・実行の全パイプラインを実行する。
///
/// # 引数
/// - `source`   : 実行するソースコード文字列（`.ar` ファイルの内容など）。
/// - `filename` : エラーメッセージのスパン情報に使用するファイル名（例: `"script.ar"` や `"<stdin>"`）。
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
fn run_program(
    source: &str,
    filename: &str,
    mut cli_args: std::collections::HashMap<String, String>,
) -> Result<(), String> {
    // `--vm` フラグを取り出す（スクリプトの `args` には渡さない）。
    let vm_mode = match cli_args.remove("__vm__").as_deref() {
        Some("off") => vm::VmMode::Off,
        Some("force") => vm::VmMode::Force,
        Some("auto") | None => vm::VmMode::Auto,
        Some(other) => {
            eprintln!("Warning: unknown --vm mode '{other}', using 'auto'");
            vm::VmMode::Auto
        }
    };
    // --- 字句解析: ソースをトークン列（Vec<Spanned>）に変換する ---
    let tokens = Lexer::new(source, filename).tokenize();

    // ソースファイルのディレクトリを解決する（import 時の検索基準）
    let source_dir = std::path::Path::new(filename)
        .parent()
        .map(|p| p.to_path_buf());

    // --- 構文解析: トークン列を AST（Vec<Stmt>）に変換する ---
    let mut stmts = Parser::new(tokens, source_dir.clone())
        .parse_program()
        .map_err(|e| format!("ParseError: {e}"))?;

    // --- 静的型検査: AST を走査してエラー・警告を収集し、AST 型解決層の注釈を生成する（#16 段階(a)） ---
    let (type_errors, type_warnings, annotations) = TypeChecker::check_program(&stmts);
    for w in &type_warnings {
        eprintln!("Warning: {w}");
    }
    if !type_errors.is_empty() {
        return Err(format_static_errors(&type_errors));
    }

    // 診断フック（#16 段階(b)(ii)）: `AR_ANNOT_DIFF=1` で境界検査指示の生成状況を出す。
    // 「Call 注釈のうち引数に CheckBefore が付いたものが何件か」を全例題で測るために使う。
    if std::env::var("AR_ANNOT_DIFF").is_ok_and(|v| !v.is_empty()) {
        let (calls, checked) = annotations.call_check_stats();
        eprintln!("AnnotCalls: calls={calls} args_with_CheckBefore={checked}");
        let m = annotations.binop_miss();
        eprintln!(
            "AnnotBinop: specialized={} miss_both_unresolved={} miss_one_unresolved={} miss_resolved_mixed={}",
            annotations.binop_kind_len(),
            m.both_unresolved,
            m.one_unresolved,
            m.resolved_but_mixed
        );
        let srcs: Vec<String> = annotations
            .unresolved_sources()
            .into_iter()
            .map(|(k, n)| format!("{k}={n}"))
            .collect();
        eprintln!("AnnotUnresolvedSrc: {}", srcs.join(" "));
    }

    // --- Phase R / R1: ローカル読み取りの slot 解決（トップレベル関数を書き換える） ---
    interpreter::resolver::resolve_program(&mut stmts);

    // --- インタープリタの初期化とソーステキストの登録 ---
    // ソーステキストはエラー報告時のスタックトレース表示に使用される
    let mut interp = Interpreter::new();
    interp.set_vm_mode(vm_mode);
    // AST 型解決層の注釈を注入する（#16）。段階(b)/(c) の消費側が node-id で参照する。
    interp.set_annotations(std::rc::Rc::new(annotations));
    interp.add_source_text(filename, source);
    // ソースファイルのディレクトリを import 検索パスに追加する
    if let Some(dir) = &source_dir {
        interp.add_python_search_dir(dir.clone());
        // ar_config.json の python.search_paths を追加する（source_dir から上位へウォーク）
        let mut walk: Option<&std::path::Path> = Some(dir.as_path());
        while let Some(d) = walk {
            let cfg_path = d.join("ar_config.json");
            if cfg_path.exists() {
                if let Ok(text) = std::fs::read_to_string(&cfg_path) {
                    if let Ok(root) = serde_json::from_str::<serde_json::Value>(&text) {
                        if let Some(paths) = root
                            .get("python")
                            .and_then(|p| p.get("search_paths"))
                            .and_then(|v| v.as_array())
                        {
                            for p in paths {
                                if let Some(s) = p.as_str() {
                                    let pb = std::path::PathBuf::from(s);
                                    let abs = if pb.is_absolute() { pb } else { d.join(pb) };
                                    interp.add_python_search_dir(abs);
                                }
                            }
                        }
                    }
                }
                break;
            }
            walk = d.parent();
        }
    }
    // CLIパラメータを `args` dict としてグローバルスコープに登録する
    interp.set_cli_args(cli_args);

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
                let msg = interp
                    .take_current_exception()
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
        Mode::Run(path, cli_args) => {
            // .arc: extract embedded source first, then run normally
            let (source, filename) = if path.ends_with(".arc") {
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
            if let Err(e) = run_program(&source, &filename, cli_args) {
                eprintln!("{e}");
                std::process::exit(1);
            }
        }

        Mode::Stdin => {
            use std::io::Read;
            let mut buf = String::new();
            std::io::stdin()
                .read_to_string(&mut buf)
                .expect("failed to read stdin");
            if let Err(e) = run_program(&buf, "<stdin>", std::collections::HashMap::new()) {
                eprintln!("{e}");
                std::process::exit(1);
            }
        }

        Mode::Compile(path) => {
            compile_module(&path);
        }

        Mode::CompileCs(path) => {
            compile_cs_stub(&path);
        }

        Mode::Repl => {
            repl::run_repl();
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

/// `--compile-cs` モード: .NET DLL を読み込んで `.ars` スタブファイルを生成する。
///
/// `import[cs-dll]` / `import[cs-proc]` が内部的に生成する型情報と同一の内容を、
/// コーダーが参照できる `.ars` ファイルとして書き出す。
/// Python の `stubgen` に相当する明示的なスタブ生成コマンド。
fn compile_cs_stub(path: &str) {
    let dll_path = std::path::Path::new(path);

    let (_, stub_text) = match parser::cs_assembly::generate_cs_stub_text(dll_path) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Error: {e}");
            std::process::exit(1);
        }
    };

    let ars_path = dll_path.with_extension("ars");
    if let Err(e) = std::fs::write(&ars_path, &stub_text) {
        eprintln!("Error writing stub '{}': {e}", ars_path.display());
        std::process::exit(1);
    }

    println!("Stub     : {}", ars_path.display());
}

/// `--compile` モード: ソースをパース・型検査して `.arc` と `.ars` を生成する。
fn compile_module(path: &str) {
    let source = read_file(path);

    let tokens = Lexer::new(&source, path).tokenize();
    let source_dir = std::path::Path::new(path).parent().map(|p| p.to_path_buf());

    let stmts = Parser::new(tokens, source_dir)
        .parse_program()
        .unwrap_or_else(|e| {
            eprintln!("ParseError: {e}");
            std::process::exit(1);
        });

    // 型検査と同時に AST 型解決層の注釈を生成する（#16 段階(c)）。
    // ネイティブ codegen はこの注釈を消費して自前の型再導出を置き換える。
    // node-id はこのモジュールのパーサが採番したもので、codegen が扱うのも同じ
    // トップレベル定義のみ（import 済みモジュールの body は `Stmt::Import` に入れ子で
    // 型検査も codegen も踏み込まない）＝ node-id 空間が一致する。
    let (type_errors, annotations) = TypeChecker::check_and_annotate(&stmts);
    if !type_errors.is_empty() {
        for e in &type_errors {
            eprintln!("{e}");
        }
        std::process::exit(1);
    }

    match partial_compiler::compile(&source, &stmts, std::path::Path::new(path), &annotations) {
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
