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

use crate::ast::Stmt;
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
        "\x1b[32mArrow REPL\x1b[0m  — Ctrl+Enter in VS Code to run selection · Ctrl+D to exit"
    );

    let mut interp = Interpreter::new();
    // #36/#33: 実行経路はバイトコード VM 一本（`--vm` もツリーウォークも無い）。
    // 解決情報はブロックごとに `run_block` が用意する
    // （`resolve_and_annotate` ＋ globals の積み増し。配線は #88 で 1 箇所に畳んだ）。
    let stdin = io::stdin();
    let mut pending: Vec<String> = Vec::new();
    // ⚠⚠ **実行し終えたブロックの AST を捨てない**（#36）。最上位 Chunk キャッシュ
    // （`Interpreter::vm_toplevel_chunks`）は **`Stmt` のアドレス**をキーにするので、
    // 解放するとアロケータが同じアドレスを再利用し、**別の文が前の文の Chunk を実行する**
    // （実際に `let xs = …` が `let total = …` の Chunk を引き当てた）。
    // `Vec` を move してもヒープ上の要素は動かないのでアドレスは保たれる。
    let mut kept_asts: Vec<Vec<Stmt>> = Vec::new();

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
                        if let Some(stmts) = run_block(&mut interp, &code) {
                            kept_asts.push(stmts);
                        }
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
/// 戻り値は実行したブロックの AST。**呼び出し側が保持し続けること**（上記の不変条件）。
fn run_block(interp: &mut Interpreter, code: &str) -> Option<Vec<Stmt>> {
    let tokens = Lexer::new(code, "<repl>").tokenize();
    let mut stmts = match Parser::new(tokens, None).parse_program() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("ParseError: {e}");
            return None;
        }
    };
    if stmts.is_empty() {
        return None;
    }
    // Register source text so tracebacks show context lines.
    interp.add_source_text("<repl>", code);

    // #36: ブロックごとに本番と同じ解決情報を用意する（`run_program` の 3 点）。
    //
    // ⚠ **グローバル名は積み増す**（`extend_`）。前のブロックで宣言した名前も
    // 「`scopes[0]` を指す」と判断できないと、後のブロックの代入が VM に載らない。
    // ⚠ **注釈はブロックごとに差し替える**。node-id はパース単位で振られるので、
    // 前のブロックの AST を後のブロックの注釈表で引くと別のノードを見る。
    // 注釈は最適化ヒントであって意味論の根拠ではない（#15e）ので、
    // 食い違っても「特化が乗らない／bail する」方向にしか倒れない。
    // ⚠ 配線は 1 箇所（#88）。⚠ この入口は**型エラーを無視して続行する**
    // （REPL は次のブロックで直せる）。⚠ #88 まで**解決 → 型検査**の順で、
    // `run_program` と逆だった（型検査が `Resolution` を読まないので無害だったが理由が無い差）。
    let (_errors, _warnings, annotations) =
        crate::interpreter::resolver::resolve_and_annotate(&mut stmts);
    interp.wire_resolution(
        annotations,
        crate::interpreter::resolver::toplevel_declared_globals(&stmts),
        crate::interpreter::GlobalsMode::Extend,
    );

    let last = stmts.len() - 1;
    for (i, stmt) in stmts.iter().enumerate() {
        match interp.exec_repl_stmt(stmt, i == last) {
            Ok(Some(out)) => println!("{out}"),
            Ok(None) => {}
            Err(e) => {
                eprintln!("{e}");
                break; // stop the block on first error（AST は返して保持させる）
            }
        }
    }
    Some(stmts)
}
