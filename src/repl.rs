// repl.rs — persistent REPL mode (`--repl` flag)
//
// Reads code blocks from stdin. Blocks are terminated by a sentinel line
// ("##REPL_EXEC##") that the VS Code extension appends after the selection.
// The interpreter is kept alive between blocks so variables and definitions
// declared in one block are visible in the next.
//
// Last-statement expression display:
//   If the final statement in a block is a bare expression whose value is
//   not None, its repr is printed — mirroring Python / Jupyter behaviour.

use std::io::{self, BufRead};

use crate::interpreter::Interpreter;
use crate::lexer::Lexer;
use crate::parser::Parser;

const EXEC_SENTINEL: &str = "##REPL_EXEC##";

/// 対話式 REPL ループを起動する。
///
/// 標準入力からコードブロックを読み込み、センチネル行 (`##REPL_EXEC##`) を受信するたびに
/// そのブロックを実行する。インタープリタは呼び出し間で維持されるため、
/// あるブロックで宣言した変数や関数は次のブロックから参照できる。
pub fn run_repl() {
    eprintln!(
        "\x1b[32mHavakyrie REPL\x1b[0m  — Ctrl+Enter in VS Code to run selection · Ctrl+D to exit"
    );

    let mut interp = Interpreter::new();
    let stdin = io::stdin();
    let mut pending: Vec<String> = Vec::new();

    loop {
        let mut line = String::new();
        match stdin.lock().read_line(&mut line) {
            Ok(0) => break, // EOF / Ctrl+D
            Ok(_) => {
                let trimmed = line.trim_end_matches(['\n', '\r']).to_string();
                if trimmed == EXEC_SENTINEL {
                    if !pending.is_empty() {
                        let code = pending.join("\n");
                        pending.clear();
                        run_block(&mut interp, &code);
                    }
                } else {
                    pending.push(trimmed);
                }
            }
            Err(_) => break,
        }
    }
}

/// 単一のコードブロックを字句解析・構文解析・実行する。
///
/// 最後の文が式文であり、その評価結果が `None` 以外の場合は標準出力に repr を出力する。
/// パースエラーまたは実行時エラーが発生した場合は標準エラー出力に表示してブロックの処理を中断する。
fn run_block(interp: &mut Interpreter, code: &str) {
    let tokens = Lexer::new(code, "<repl>").tokenize();
    let stmts = match Parser::new(tokens, None).parse_program() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("ParseError: {e}");
            return;
        }
    };
    if stmts.is_empty() {
        return;
    }
    // Register source text so tracebacks show context lines.
    interp.add_source_text("<repl>", code);

    let last = stmts.len() - 1;
    for (i, stmt) in stmts.iter().enumerate() {
        match interp.exec_repl_stmt(stmt, i == last) {
            Ok(Some(out)) => println!("{out}"),
            Ok(None) => {}
            Err(e) => {
                eprintln!("{e}");
                return; // stop the block on first error
            }
        }
    }
}
