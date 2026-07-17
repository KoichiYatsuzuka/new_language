// tests/events_external.rs — 外部イベント発火 (ar_event_fire → external_handler_registry) のテスト。
//
// 注意: GLOBAL_EXT_QUEUE はプロセス全体で共有される (OnceLock)。
// 他のテストとの干渉を避けるため、外部キューに触るテストはこの 1 本にまとめる。

use super::*;
use crate::interpreter::*;

/// `sig.external_id` で発番 → 別スレッドから `ar_event_fire` → `EventLoop.run(timeout)` で
/// ハンドラがペイロードを受け取る一連の経路を検証する。
#[test]
fn test_external_event_fire_end_to_end() {
    let src = "
mut received = \"\"
mut count = 0
let sig = Signal[str]()
fn on_msg(mut msg: str) -> None:
    received = msg
    count = count + 1
sig on on_msg
let eid = sig.external_id
let eid2 = sig.external_id
";
    let tokens = Lexer::new(src, "").tokenize();
    let stmts = Parser::new(tokens, None).parse_program().unwrap();
    let mut interp = Interpreter::new();
    for stmt in &stmts {
        let _ = interp.exec(stmt).unwrap();
    }

    // external_id は初回発番後、同じ値を返し続ける
    let id = match interp.get_val("eid").unwrap() {
        Value::Int(n) => n,
        other => panic!("expected Int external_id, got {:?}", other),
    };
    assert!(id >= 1);
    assert_int(interp.get_val("eid2").unwrap(), id);

    // 別スレッドから C ABI 経由で発火する (C#/Go ブリッジと同じ入口)
    let fire_id = id as u64;
    let th = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(50));
        let payload = b"hello";
        unsafe { event_loop::ar_event_fire(fire_id, payload.as_ptr(), payload.len()) };
    });

    // タイムアウト付き run でないと外部イベントを待たずに即終了する点に注意
    let src2 = "EventLoop.run(0.5)\n";
    let tokens = Lexer::new(src2, "").tokenize();
    let stmts = Parser::new(tokens, None).parse_program().unwrap();
    for stmt in &stmts {
        let _ = interp.exec(stmt).unwrap();
    }
    th.join().unwrap();

    assert_str(interp.get_val("received").unwrap(), "hello");
    assert_int(interp.get_val("count").unwrap(), 1);
}
