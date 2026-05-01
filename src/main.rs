mod ast;
mod interpreter;
mod lexer;
mod parser;
mod token;

use interpreter::Interpreter;
use lexer::Lexer;
use parser::Parser;

fn main() {
    let source = if let Some(path) = std::env::args().nth(1) {
        std::fs::read_to_string(&path).unwrap_or_else(|e| {
            eprintln!("Error reading {path}: {e}");
            std::process::exit(1);
        })
    } else {
        use std::io::Read;
        let mut buf = String::new();
        std::io::stdin().read_to_string(&mut buf).expect("failed to read stdin");
        buf
    };

    let tokens = Lexer::new(&source).tokenize();

    let stmts = Parser::new(tokens).parse_program().unwrap_or_else(|e| {
        eprintln!("ParseError: {e}");
        std::process::exit(1);
    });

    let mut interp = Interpreter::new();
    for stmt in &stmts {
        if let Err(e) = interp.exec(stmt) {
            eprintln!("{e}");
            std::process::exit(1);
        }
    }
}
