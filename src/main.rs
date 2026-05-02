mod ast;
mod interpreter;
mod lexer;
mod parser;
mod token;
mod type_check;

use interpreter::Interpreter;
use lexer::Lexer;
use parser::Parser;
use type_check::TypeChecker;

fn parse_args() -> Option<String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < args.len() {
        if args[i] == "-src" {
            return args.get(i + 1).cloned().or_else(|| {
                eprintln!("Error: -src requires a file path");
                std::process::exit(1);
            });
        }
        i += 1;
    }
    // Positional fallback: first argument that doesn't start with '-'
    args.into_iter().find(|a| !a.starts_with('-'))
}

fn main() {
    let (source, filename) = if let Some(path) = parse_args() {
        let content = std::fs::read_to_string(&path).unwrap_or_else(|e| {
            eprintln!("Error reading {path}: {e}");
            std::process::exit(1);
        });
        (content, path)
    } else {
        use std::io::Read;
        let mut buf = String::new();
        std::io::stdin().read_to_string(&mut buf).expect("failed to read stdin");
        (buf, "<stdin>".to_string())
    };

    let tokens = Lexer::new(&source, filename.as_str()).tokenize();

    let stmts = Parser::new(tokens).parse_program().unwrap_or_else(|e| {
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

    let mut interp = Interpreter::new();
    for stmt in &stmts {
        if let Err(e) = interp.exec(stmt) {
            eprintln!("{e}");
            std::process::exit(1);
        }
    }
}
