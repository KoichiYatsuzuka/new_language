// tests/file_io.rs — ファイル I/O のテスト。

use super::*;
use crate::interpreter::*;

// ---------------------------------------------------------------------------
// ファイル I/O テスト
// ---------------------------------------------------------------------------

/// テスト用の一時ファイルパスを生成する（ユニークなサフィックス付き）。
fn temp_path(suffix: &str) -> String {
    format!("target/test_tmp_{suffix}.txt")
}

/// テスト後に一時ファイルを削除する。
fn cleanup(path: &str) {
    let _ = std::fs::remove_file(path);
}

/// file_open_mode_enum のテスト。
#[test]
fn test_file_open_mode_enum() {
    // FileOpenMode enum が正しくグローバルスコープに登録されているかを確認する
    let src = "let m = FileOpenMode.read\nlet v = m.value\n";
    let val = run_get(src, "v");
    assert!(
        matches!(val, Value::Int(2)),
        "FileOpenMode.read.value should be 2"
    );
}

/// file_start_point_enum のテスト。
#[test]
fn test_file_start_point_enum() {
    let src = "let s = StartPoint.end\nlet v = s.value\n";
    let val = run_get(src, "v");
    assert!(
        matches!(val, Value::Int(1)),
        "StartPoint.end.value should be 1"
    );
}

/// file_path_type のテスト。
#[test]
fn test_file_path_type() {
    // path 型のインスタンスを生成できることを確認する
    let src = "let p = path(\"foo.txt\")\nlet v = p.value\n";
    let val = run_get(src, "v");
    assert!(
        matches!(val, Value::Str(s) if s == "foo.txt"),
        "path.value should be 'foo.txt'"
    );
}

/// file_rewrite_and_read のテスト。
#[test]
fn test_file_rewrite_and_read() {
    let p = temp_path("rewrite_read");
    cleanup(&p);
    let src = format!(
        "let f = open(\"{p}\", FileOpenMode.rewrite)\nf.write(\"hello\")\nclose(f)\n\
         let g = open(\"{p}\", FileOpenMode.read)\nlet r = g.read()\nclose(g)\n",
    );
    run(&src).expect("file rewrite + read should succeed");
    let src2 = format!("let g = open(\"{p}\", FileOpenMode.read)\nlet r = g.read()\nclose(g)\n");
    let val = run_get(&src2, "r");
    cleanup(&p);
    assert!(
        matches!(val, Value::Str(s) if s == "hello"),
        "read() should return written text"
    );
}

/// file_write_line のテスト。
#[test]
fn test_file_write_line() {
    let p = temp_path("write_line");
    cleanup(&p);
    let src = format!(
        "let f = open(\"{p}\", FileOpenMode.rewrite)\n\
         f.write_line(\"line1\")\nf.write_line(\"line2\")\nclose(f)\n"
    );
    run(&src).expect("write_line should succeed");
    let src2 = format!("let g = open(\"{p}\", FileOpenMode.read)\nlet r = g.read()\nclose(g)\n");
    let val = run_get(&src2, "r");
    cleanup(&p);
    assert!(
        matches!(val, Value::Str(s) if s == "line1\nline2\n"),
        "write_line should append newline"
    );
}

/// file_read_line_forward のテスト。
#[test]
fn test_file_read_line_forward() {
    let p = temp_path("read_line_fwd");
    cleanup(&p);
    std::fs::write(&p, "alpha\nbeta\n").unwrap();
    let src = format!(
        "let f = open(\"{p}\", FileOpenMode.read)\n\
         let a = f.read_line()\nlet b = f.read_line()\nclose(f)\n"
    );
    let tokens = crate::lexer::Lexer::new(&src, "").tokenize();
    let stmts = crate::parser::Parser::new(tokens, None)
        .parse_program()
        .unwrap();
    let mut interp = Interpreter::new();
    for stmt in &stmts {
        interp.exec(stmt).unwrap();
    }
    let a = interp.get_val("a").unwrap();
    let b = interp.get_val("b").unwrap();
    cleanup(&p);
    assert!(
        matches!(a, Value::Str(s) if s == "alpha\n"),
        "first read_line should be 'alpha\\n'"
    );
    assert!(
        matches!(b, Value::Str(s) if s == "beta\n"),
        "second read_line should be 'beta\\n'"
    );
}

/// file_read_letter のテスト。
#[test]
fn test_file_read_letter() {
    let p = temp_path("read_letter");
    cleanup(&p);
    std::fs::write(&p, "AB").unwrap();
    let src = format!(
        "let f = open(\"{p}\", FileOpenMode.read)\n\
         let a = f.read_letter()\nlet b = f.read_letter()\nclose(f)\n"
    );
    let tokens = crate::lexer::Lexer::new(&src, "").tokenize();
    let stmts = crate::parser::Parser::new(tokens, None)
        .parse_program()
        .unwrap();
    let mut interp = Interpreter::new();
    for stmt in &stmts {
        interp.exec(stmt).unwrap();
    }
    let a = interp.get_val("a").unwrap();
    let b = interp.get_val("b").unwrap();
    cleanup(&p);
    assert!(
        matches!(a, Value::Str(s) if s == "A"),
        "first letter should be 'A'"
    );
    assert!(
        matches!(b, Value::Str(s) if s == "B"),
        "second letter should be 'B'"
    );
}

/// file_eof_error のテスト。
#[test]
fn test_file_eof_error() {
    let p = temp_path("eof");
    cleanup(&p);
    std::fs::write(&p, "x").unwrap();
    let src = format!(
        "let f = open(\"{p}\", FileOpenMode.read, StartPoint.end)\nlet r = f.read()\nclose(f)\n"
    );
    assert!(run(&src).is_err(), "read at EOF should raise EOFError");
    cleanup(&p);
}

/// file_bof_error のテスト。
#[test]
fn test_file_bof_error() {
    let p = temp_path("bof");
    cleanup(&p);
    std::fs::write(&p, "x").unwrap();
    let src = format!(
        "let f = open(\"{p}\", FileOpenMode.read)\nlet r = f.read(backward = True)\nclose(f)\n"
    );
    assert!(run(&src).is_err(), "read at BOF should raise BOFError");
    cleanup(&p);
}

/// file_make_and_write_existing_error のテスト。
#[test]
fn test_file_make_and_write_existing_error() {
    let p = temp_path("maw_exist");
    std::fs::write(&p, "existing").unwrap();
    let src = format!("let f = open(\"{p}\", FileOpenMode.make_and_write)\nclose(f)\n");
    assert!(
        run(&src).is_err(),
        "make_and_write on existing file should error"
    );
    cleanup(&p);
}

/// file_write_read_only_error のテスト。
#[test]
fn test_file_write_read_only_error() {
    let p = temp_path("write_ro");
    cleanup(&p);
    std::fs::write(&p, "hello").unwrap();
    let src = format!("let f = open(\"{p}\", FileOpenMode.read)\nf.write(\"x\")\nclose(f)\n");
    assert!(run(&src).is_err(), "write on read-only file should error");
    cleanup(&p);
}

/// file_insert_midpoint のテスト。
#[test]
fn test_file_insert_midpoint() {
    let p = temp_path("insert_mid");
    cleanup(&p);
    // Write "helo", then open and insert "l" at position 3 → "hello"
    std::fs::write(&p, "helo").unwrap();
    let src = format!(
        "let f = open(\"{p}\", FileOpenMode.write)\n\
         let _ = f.read_letter()\nlet _ = f.read_letter()\nlet _ = f.read_letter()\n\
         f.write(\"l\")\nclose(f)\n"
    );
    run(&src).expect("insert mid should succeed");
    let content = std::fs::read_to_string(&p).unwrap();
    cleanup(&p);
    assert_eq!(
        content, "hello",
        "inserting 'l' at position 3 should give 'hello'"
    );
}

/// file_byte_mode_write_read のテスト。
#[test]
fn test_file_byte_mode_write_read() {
    let p = temp_path("byte_mode");
    cleanup(&p);
    let src = format!(
        "let f = open(\"{p}\", FileOpenMode.rewrite, StartPoint.top, ByteRecognizingMode.byte)\n\
         f.write([72, 105])\nclose(f)\n\
         let g = open(\"{p}\", FileOpenMode.read, StartPoint.top, ByteRecognizingMode.byte)\n\
         let r = g.read()\nclose(g)\n"
    );
    let tokens = crate::lexer::Lexer::new(&src, "").tokenize();
    let stmts = crate::parser::Parser::new(tokens, None)
        .parse_program()
        .unwrap();
    let mut interp = Interpreter::new();
    for stmt in &stmts {
        interp.exec(stmt).unwrap();
    }
    let val = interp.get_val("r").unwrap();
    cleanup(&p);
    // r should be [72, 105]
    if let Value::List(items) = val {
        let items = items.borrow();
        assert_eq!(items.len(), 2);
        assert!(matches!(items[0], Value::Int(72)));
        assert!(matches!(items[1], Value::Int(105)));
    } else {
        panic!("expected list of bytes");
    }
}

