// tests/exceptions.rs — 例外処理(try/except/finally/raise)と内部エラーの捕捉可能性のテスト。

use super::*;

// --- Exception handling tests ---

/// raise_uncaught_reaches_caller のテスト。
#[test]
fn test_raise_uncaught_reaches_caller() {
    let raised = run_exc("raise ValueError(\"oops\")\n").unwrap();
    let raised = raised.expect("expected a raised exception");
    if let Value::Instance(inst) = &raised.exception {
        assert_eq!(inst.borrow().class.name, "ValueError");
        let b = inst.borrow();
        let msg = b.class.field_index.get("message").and_then(|&idx| {
            match b.field_value(idx) {
                Some(Value::Str(s)) => Some(s),
                _ => None,
            }
        }).expect("message field missing or wrong type");
        drop(b);
        assert_eq!(&*msg, "oops");
    } else {
        panic!("expected Instance");
    }
}

/// try_except_catches_matching_type のテスト。
#[test]
fn test_try_except_catches_matching_type() {
    let src = concat!(
        "mut x = 0\n",
        "try:\n",
        "    raise ValueError(\"bad\")\n",
        "except ValueError as e:\n",
        "    x = 1\n",
    );
    assert!(run(src).is_ok());
    let x = run_get(src, "x");
    assert!(matches!(x, Value::Int(1)));
}

/// try_except_does_not_catch_different_type のテスト。
#[test]
fn test_try_except_does_not_catch_different_type() {
    let src = concat!(
        "try:\n",
        "    raise TypeError(\"t\")\n",
        "except ValueError as e:\n",
        "    pass\n",
    );
    let raised = run_exc(src).unwrap();
    assert!(
        raised.is_some(),
        "TypeError should not be caught by ValueError handler"
    );
    if let Some(r) = raised {
        if let Value::Instance(inst) = &r.exception {
            assert_eq!(inst.borrow().class.name, "TypeError");
        }
    }
}

/// try_finally_runs_always のテスト。
#[test]
fn test_try_finally_runs_always() {
    let src = concat!(
        "mut x = 0\n",
        "try:\n",
        "    raise RuntimeError(\"r\")\n",
        "except RuntimeError:\n",
        "    x = 1\n",
        "finally:\n",
        "    x = 2\n",
    );
    let x = run_get(src, "x");
    assert!(matches!(x, Value::Int(2)));
}

/// try_no_raise_skips_except のテスト。
#[test]
fn test_try_no_raise_skips_except() {
    let src = concat!(
        "mut x = 0\n",
        "try:\n",
        "    x = 5\n",
        "except ValueError:\n",
        "    x = 99\n",
    );
    let x = run_get(src, "x");
    assert!(matches!(x, Value::Int(5)));
}

/// try_bare_except_catches_all のテスト。
#[test]
fn test_try_bare_except_catches_all() {
    let src = concat!(
        "mut caught = 0\n",
        "try:\n",
        "    raise KeyError(\"k\")\n",
        "except:\n",
        "    caught = 1\n",
    );
    let v = run_get(src, "caught");
    assert!(matches!(v, Value::Int(1)));
}

/// user_defined_error_class のテスト。
#[test]
fn test_user_defined_error_class() {
    let src = concat!(
        "class MyError(Error):\n",
        "    pass\n",
        "mut caught = 0\n",
        "try:\n",
        "    raise MyError(\"custom\")\n",
        "except MyError as e:\n",
        "    caught = 1\n",
    );
    let v = run_get(src, "caught");
    assert!(matches!(v, Value::Int(1)));
}

/// exception_message_accessible のテスト。
#[test]
fn test_exception_message_accessible() {
    let src = concat!(
        "mut msg = \"\"\n",
        "try:\n",
        "    raise ValueError(\"hello world\")\n",
        "except ValueError as e:\n",
        "    msg = e.message\n",
    );
    let v = run_get(src, "msg");
    match v {
        Value::Str(s) => assert_eq!(&*s, "hello world"),
        _ => panic!("expected Str"),
    }
}

/// bare_raise_reraises のテスト。
#[test]
fn test_bare_raise_reraises() {
    let src = concat!(
        "mut x = 0\n",
        "try:\n",
        "    try:\n",
        "        raise ValueError(\"v\")\n",
        "    except ValueError as e:\n",
        "        x = 1\n",
        "        raise\n",
        "except ValueError:\n",
        "    x = 2\n",
    );
    let v = run_get(src, "x");
    assert!(matches!(v, Value::Int(2)));
}

/// exception_propagates_through_function のテスト。
#[test]
fn test_exception_propagates_through_function() {
    let src = concat!(
        "fn thrower() -> None:\n",
        "    raise RuntimeError(\"from fn\")\n",
        "mut caught = 0\n",
        "try:\n",
        "    thrower()\n",
        "except RuntimeError:\n",
        "    caught = 1\n",
    );
    let v = run_get(src, "caught");
    assert!(matches!(v, Value::Int(1)));
}

// --- internal error catchability ---

/// catch_internal_type_error のテスト。
#[test]
fn test_catch_internal_type_error() {
    let src = concat!(
        "mut caught = 0\n",
        "try:\n",
        "    let x: int = 1 + \"bad\"\n",
        "except TypeError:\n",
        "    caught = 1\n",
    );
    let v = run_get(src, "caught");
    assert!(matches!(v, Value::Int(1)));
}

/// catch_internal_index_error のテスト。
#[test]
fn test_catch_internal_index_error() {
    let src = concat!(
        "mut caught = 0\n",
        "let lst = [1, 2, 3]\n",
        "try:\n",
        "    let x = lst[10]\n",
        "except IndexError:\n",
        "    caught = 1\n",
    );
    let v = run_get(src, "caught");
    assert!(matches!(v, Value::Int(1)));
}

/// catch_internal_key_error のテスト。
#[test]
fn test_catch_internal_key_error() {
    let src = concat!(
        "mut caught = 0\n",
        "let d = {\"a\": 1}\n",
        "try:\n",
        "    let x = d[\"missing\"]\n",
        "except KeyError:\n",
        "    caught = 1\n",
    );
    let v = run_get(src, "caught");
    assert!(matches!(v, Value::Int(1)));
}

/// catch_internal_zero_division のテスト。
#[test]
fn test_catch_internal_zero_division() {
    let src = concat!(
        "mut caught = 0\n",
        "try:\n",
        "    let x = 10 / 0\n",
        "except ZeroDivisionError:\n",
        "    caught = 1\n",
    );
    let v = run_get(src, "caught");
    assert!(matches!(v, Value::Int(1)));
}

/// catch_internal_name_error のテスト。
#[test]
fn test_catch_internal_name_error() {
    let src = concat!(
        "mut caught = 0\n",
        "try:\n",
        "    let x = undefined_variable\n",
        "except NameError:\n",
        "    caught = 1\n",
    );
    let v = run_get(src, "caught");
    assert!(matches!(v, Value::Int(1)));
}

/// internal_error_message_accessible のテスト。
#[test]
fn test_internal_error_message_accessible() {
    let src = concat!(
        "mut msg = \"\"\n",
        "let lst = [1, 2]\n",
        "try:\n",
        "    let x = lst[99]\n",
        "except IndexError as e:\n",
        "    msg = e.message\n",
    );
    let v = run_get(src, "msg");
    match v {
        Value::Str(s) => assert!(!s.is_empty(), "message should be non-empty"),
        _ => panic!("expected Str"),
    }
}

/// bare_except_catches_internal_error のテスト。
#[test]
fn test_bare_except_catches_internal_error() {
    let src = concat!(
        "mut caught = 0\n",
        "try:\n",
        "    let x = 1 / 0\n",
        "except:\n",
        "    caught = 1\n",
    );
    let v = run_get(src, "caught");
    assert!(matches!(v, Value::Int(1)));
}

/// internal_error_not_caught_by_wrong_type のテスト。
#[test]
fn test_internal_error_not_caught_by_wrong_type() {
    let src = concat!(
        "try:\n",
        "    let x = 1 / 0\n",
        "except ValueError:\n",
        "    pass\n",
    );
    let raised = run_exc(src).unwrap();
    assert!(
        raised.is_some(),
        "ZeroDivisionError should not be caught by ValueError handler"
    );
}

/// internal_error_in_function_is_catchable のテスト。
#[test]
fn test_internal_error_in_function_is_catchable() {
    let src = concat!(
        "fn divide(a: int, b: int) -> int:\n",
        "    return a / b\n",
        "mut caught = 0\n",
        "try:\n",
        "    divide(10, 0)\n",
        "except ZeroDivisionError:\n",
        "    caught = 1\n",
    );
    let v = run_get(src, "caught");
    assert!(matches!(v, Value::Int(1)));
}

/// finally_runs_after_internal_error のテスト。
#[test]
fn test_finally_runs_after_internal_error() {
    let src = concat!(
        "mut x = 0\n",
        "try:\n",
        "    let y = 1 / 0\n",
        "except ZeroDivisionError:\n",
        "    x = 1\n",
        "finally:\n",
        "    x = 2\n",
    );
    let v = run_get(src, "x");
    assert!(matches!(v, Value::Int(2)));
}


/// 関数の中で起きた誤りの traceback の最も内側のフレームに、失敗した文の位置が付く（フェーズ10 10-17）。
/// ⚠ 以前は `File "", in inner` で、どの行で落ちたかが出なかった。
#[test]
fn test_innermost_frame_has_failing_statement_location() {
    let src = concat!(
        "fn inner(let x: int) -> int:\n",
        "    let d: dict[str, int] = {}\n",
        "    let v = d[\"k\"] + x\n",
        "    return v\n",
        "let r = inner(1)\n",
    );
    let raised = run_exc(src).expect("run").expect("raised");
    let innermost = raised.frames.first().expect("frame");
    assert_eq!(innermost.fn_name, "inner");
    assert_eq!(innermost.line, 3, "{:?}", raised.frames);
}


/// `return` の式で起きた誤りも最も内側のフレームに行が付く（フェーズ10 10-17）。
/// ⚠ 以前は `return` が位置を持たず、行テーブルに載らないので `File "", in inner` だった。
#[test]
fn test_innermost_frame_location_for_return_statement() {
    let src = concat!(
        "fn inner(let x: int) -> int:\n",
        "    let d: dict[str, int] = {}\n",
        "    return d[\"k\"] + x\n",
        "let r = inner(1)\n",
    );
    let raised = run_exc(src).expect("run").expect("raised");
    let innermost = raised.frames.first().expect("frame");
    assert_eq!(innermost.fn_name, "inner");
    assert_eq!(innermost.line, 3, "{:?}", raised.frames);
}

/// `except Exception` は組み込みの例外も利用者の例外も捕まえる（フェーズ10 10-19）。
/// ⚠ 以前は組み込みの例外の基底が `Error` だけで、`except Exception` は `Exception` そのものしか捕まえなかった。
#[test]
fn test_except_exception_catches_every_exception() {
    let src = concat!(
        "class MyErr(Error):\n",
        "    fn __init__(mut self, message: str) -> None:\n",
        "        self.message = message\n",
        "mut caught = 0\n",
        "try:\n",
        "    raise RuntimeError(\"r\")\n",
        "except Exception as e:\n",
        "    caught = caught + 1\n",
        "try:\n",
        "    raise MyErr(\"m\")\n",
        "except Exception as e:\n",
        "    caught = caught + 1\n",
        "try:\n",
        "    let d: dict[str, int] = {}\n",
        "    let v = d[\"k\"]\n",
        "except Exception as e:\n",
        "    caught = caught + 1\n",
    );
    let v = run_get(src, "caught");
    assert!(matches!(v, Value::Int(3)), "{v:?}");
}

/// 例外の `str(e)` / `repr(e)` / リストの中の表示（ffi_value_exchange_plan.md 7-3・Python と同じ）。
/// ⚠ 以前は `<ValueError object at 0x…>` で、アドレスが実行のたびに変わった。
#[test]
fn test_exception_str_is_its_message() {
    let src = concat!(
        "mut s = \"\"\n",
        "mut r = \"\"\n",
        "mut l = \"\"\n",
        "try:\n",
        "    raise ValueError(\"bad value\")\n",
        "except ValueError as e:\n",
        "    s = str(e)\n",
        "    r = repr(e)\n",
        "    l = str([e])\n",
    );
    let interp = run_interp(src);
    let get = |name: &str| match interp.get_val(name) {
        Some(Value::Str(s)) => s.to_string(),
        other => panic!("{name}: expected a str, got {other:?}"),
    };
    assert_eq!(get("s"), "bad value");
    assert_eq!(get("r"), "ValueError('bad value')");
    assert_eq!(get("l"), "[ValueError('bad value')]");
}

/// 外部言語との境界の例外（ffi_value_exchange_plan.md 7-4）。`FfiTypeError` は `TypeError` の派生、どちらも
/// `except Exception` で捕まえられる。
#[test]
fn test_ffi_exceptions_are_catchable() {
    let src = concat!(
        "mut caught = 0\n",
        "try:\n",
        "    raise FfiTypeError(\"f\")\n",
        "except TypeError as e:\n",
        "    caught = caught + 1\n",
        "try:\n",
        "    raise ForeignError(\"System.InvalidOperationException: boom\")\n",
        "except ForeignError as e:\n",
        "    caught = caught + 1\n",
        "try:\n",
        "    raise FfiTypeError(\"g\")\n",
        "except Exception as e:\n",
        "    caught = caught + 1\n",
    );
    let v = run_get(src, "caught");
    assert!(matches!(v, Value::Int(3)), "{v:?}");
}

/// ブリッジが返す内部エラーの文字列が、組み込みの例外の一覧から例外になる（7-4。以前は写しの表 `CATCHABLE` に
/// `FfiTypeError` / `ForeignError` が無く、`except` で捕まえられなかった）。
#[test]
fn test_bridge_error_strings_become_exceptions() {
    let mut interp = run_interp("let x = 1\n");
    for (msg, class) in [
        ("ForeignError: System.InvalidOperationException: boom", "ForeignError"),
        ("FfiTypeError: javascript function `f` is declared to return `int` but returned `str`", "FfiTypeError"),
        ("TypeError: x", "TypeError"),
    ] {
        let raised = interp.make_internal_raised_error(msg).unwrap_or_else(|| panic!("not an exception: {msg}"));
        match &raised.exception {
            Value::Instance(i) => assert_eq!(i.borrow().class.name, class, "{msg}"),
            other => panic!("{msg}: expected an exception instance, got {other:?}"),
        }
    }
    // ブリッジの基盤のエラー（接頭辞が組み込みの例外でない）はそのまま
    assert!(interp.make_internal_raised_error("CsDll: cannot load bridge 'x'").is_none());
}

/// `OSError(errno, ..)` は errno に対応する派生を作る（別名の `IOError` も・CPython の `OSError.__new__`・
/// known_bugs.md の #9）。派生を呼んだとき・引数が 1 つ・表に無い errno は呼んだクラスのまま。
#[test]
fn test_os_error_errno_picks_subclass() {
    let src = concat!(
        "let a = OSError(2, \"x\")\n",
        "let b = IOError(13, \"y\", \"f\")\n",
        "let c = PermissionError(2, \"z\")\n",
        "let d = OSError(2)\n",
        "let e = OSError(12345, \"w\")\n",
        "let names = type(a).__name__ + \",\" + type(b).__name__ + \",\" + type(c).__name__ + \",\" + type(d).__name__ + \",\" + type(e).__name__\n",
        "let shown = str(b)\n",
    );
    let interp = run_interp(src);
    assert!(
        matches!(interp.get_val("names"), Some(Value::Str(s)) if &*s == "FileNotFoundError,PermissionError,PermissionError,OSError,OSError"),
        "{:?}",
        interp.get_val("names")
    );
    assert!(matches!(interp.get_val("shown"), Some(Value::Str(s)) if &*s == "[Errno 13] y: 'f'"));
}

/// 送出の場が添えた `args`（`error_with_args`・known_bugs.md の #9）は、文言が完全に一致するときだけ使う。
/// 一致しない誤り（Rust の中で捨てた誤りの控えが残っていた）は文言の `args` のままで、控えはどちらでも取り去る。
#[test]
fn test_error_args_are_used_only_for_the_same_message() {
    let mut interp = run_interp("let x = 1\n");
    let args_of = |interp: &mut Interpreter, msg: &str| -> String {
        let raised = interp.make_internal_raised_error(msg).expect("an exception");
        let Value::Instance(i) = &raised.exception else { panic!("{msg}") };
        let idx = i.borrow().class.field_index["args"];
        let args = i.borrow().field_value(idx).expect("args");
        interp.display_repr(&args)
    };
    let msg = interp.key_error(&Value::Int(7));
    assert_eq!(msg, "KeyError: 7");
    assert_eq!(args_of(&mut interp, &msg), "(7,)", "the key itself");
    // 控えは取り去られている（同じ文言でも、もう一度は使わない）
    assert_eq!(args_of(&mut interp, &msg), "('7',)");
    // 別の文言の誤りは控えを使わず、控えも残さない
    let _ = interp.key_error(&Value::str("a"));
    assert_eq!(args_of(&mut interp, "KeyError: 'b'"), "(\"'b'\",)");
    assert!(interp.error_args.is_none());
}

/// 組み込みの例外は CPython 3.12 の階層を持つ（python_builtins_plan.md のタスク 4-1）。
/// ⚠ 以前は階層が無く、`except ArithmeticError` が `ZeroDivisionError` を捕まえなかった。
/// ⚠ `GeneratorExit` は `BaseException` の直下なので `except Exception` では捕まらない。
#[test]
fn test_builtin_exception_hierarchy() {
    let src = concat!(
        "mut caught = 0\n",
        "try:\n",
        "    let z = 1 // 0\n",
        "except ArithmeticError:\n",
        "    caught = caught + 1\n",
        "try:\n",
        "    let d: dict[str, int] = {}\n",
        "    let v = d[\"k\"]\n",
        "except LookupError:\n",
        "    caught = caught + 1\n",
        "try:\n",
        "    raise OSError(\"o\")\n",
        "except IOError:\n",
        "    caught = caught + 1\n",
        "try:\n",
        "    raise ConnectionResetError(\"c\")\n",
        "except OSError:\n",
        "    caught = caught + 1\n",
        "try:\n",
        "    try:\n",
        "        raise GeneratorExit(\"g\")\n",
        "    except Exception:\n",
        "        caught = caught + 100\n",
        "except BaseException:\n",
        "    caught = caught + 1\n",
    );
    let v = run_get(src, "caught");
    assert!(matches!(v, Value::Int(5)), "{v:?}");
}

/// `open` の失敗は OS のエラーの種類で `OSError` の派生に振り分ける（タスク 4-1・以前はどれも `IOError`）。
#[test]
fn test_open_failure_is_file_not_found_error() {
    let msg = run_err_msg("let f = open(\"__surely_missing_dir__/x.txt\", FileOpenMode.read)\n");
    assert!(msg.starts_with("FileNotFoundError"), "{msg}");
}

/// 組み込みの例外は位置引数をいくつでも受け、`args` と CPython の `str(e)` / `repr(e)` を持つ（タスク 4-2）。
/// ⚠ 以前は `ValueError()` / `ValueError("a", 1)` が作れず、`str(e)` は `<ValueError object at 0x..>` だった。
#[test]
fn test_exception_args_str_repr() {
    let src = concat!(
        "let a = ValueError()\n",
        "let c = ValueError(\"bad\", 42)\n",
        "let k = KeyError(\"m\")\n",
        "let s = \"[\" + str(a) + \"]\" + str(c) + str(k) + str(ValueError(5))\n",
        "let r = repr(a) + repr(c) + repr(k)\n",
        "let n = len(c.args)\n",
    );
    assert_str(run_get(src, "s"), "[]('bad', 42)'m'5");
    assert_str(run_get(src, "r"), "ValueError()ValueError('bad', 42)KeyError('m')");
    assert!(matches!(run_get(src, "n"), Value::Int(2)));
    assert!(run_err_msg("let e = ValueError(message=\"x\")\n")
        .contains("ValueError() takes no keyword arguments"));
}


/// Arrow のファイルの `EOFError` / `BOFError` を捕まえられる（python_builtins_plan.md のタスク 4-5）。以前は例外の表に無い
/// 名前だったので、`except Exception` も素の `except:` も素通りしてプログラムが止まった。
#[test]
fn test_file_eof_and_bof_are_catchable() {
    let path = "target/test_tmp_eof_bof.txt";
    let src = format!(
        concat!(
            "let w = open(\"{p}\", FileOpenMode.rewrite)\n",
            "w.write(\"x\")\n",
            "w.close()\n",
            "let r = open(\"{p}\", FileOpenMode.read)\n",
            "let s = r.read()\n",
            "mut eof = \"\"\n",
            "try:\n",
            "    r.read()\n",
            "except EOFError as e:\n",
            "    eof = type(e).__name__ + \":\" + str(e)\n",
            "let back = r.read(backward=True)\n",
            "mut bof = \"\"\n",
            "try:\n",
            "    r.read(backward=True)\n",
            "except BOFError as e:\n",
            "    bof = type(e).__name__\n",
            "mut generic = false\n",
            "try:\n",
            "    let t = r.read()\n",
            "    r.read()\n",
            "except:\n",
            "    generic = true\n",
        ),
        p = path
    );
    let src = src.replace("false", "False").replace("true", "True");
    let interp = run_interp(&src);
    assert!(matches!(interp.get_val("eof"), Some(Value::Str(s)) if &*s == "EOFError:EOF"));
    assert!(matches!(interp.get_val("bof"), Some(Value::Str(s)) if &*s == "BOFError"));
    assert!(matches!(interp.get_val("generic"), Some(Value::Bool(true))));
    let _ = std::fs::remove_file(path);
}