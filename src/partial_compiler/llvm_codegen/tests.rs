use super::generate_llvm_module;
use crate::lexer::Lexer;
use crate::parser::Parser;
use crate::type_check::AstAnnotations;

/// オーバーロード（明示の `__init__` と自動生成の `__init__`・同名のメソッド・同名の最上位関数）はコンパイルせず、
/// ほかの関数は今までどおりコンパイルする（native_object_access_plan.md 1-8・§0.3 ⑨）。
/// ⚠ 以前は同じ `define` が 2 つでき、clang が落ちてモジュール全体がネイティブにならなかった。
#[test]
fn overloads_are_left_to_the_interpreter() {
    let src = "\
class Box:
    mut w: int
    mut h: int

    fn __init__(mut self, let side: int) -> None:
        self.w = side
        self.h = side

    fn area(let self) -> int:
        return self.w * self.h

    fn scale(mut self, let k: int) -> None:
        self.w = self.w * k

    fn scale(mut self, let kw: int, let kh: int) -> None:
        self.w = self.w * kw
        self.h = self.h * kh

fn twice(let x: int) -> int:
    return x * 2

fn twice(let x: float) -> float:
    return x * 2.0

fn plain(let x: int) -> int:
    return x + 1
";
    let tokens = Lexer::new(src, "").tokenize();
    let stmts = Parser::new(tokens, None).parse_program().expect("parse error");
    let (ir, exports) = generate_llvm_module(&stmts, &AstAnnotations::default()).expect("eligible functions");

    let mut defined = std::collections::HashSet::new();
    for line in ir.lines().filter(|l| l.starts_with("define ")) {
        let sym = line.split('@').nth(1).and_then(|s| s.split('(').next()).expect("symbol");
        assert!(defined.insert(sym.to_string()), "duplicate definition of {sym}");
    }
    assert!(defined.contains("plain_impl") && defined.contains("Box__area_impl"), "{defined:?}");
    for overloaded in ["twice_impl", "Box____init___impl", "Box__scale_impl"] {
        assert!(!defined.contains(overloaded), "{overloaded} must not be compiled: {defined:?}");
    }
    let names: Vec<&str> = exports.iter().map(|e| e.name.as_str()).collect();
    assert!(names.contains(&"plain") && names.contains(&"area"), "{names:?}");
    assert!(!names.contains(&"twice") && !names.contains(&"__init__") && !names.contains(&"scale"), "{names:?}");
}

/// 大域へ代入・複合代入する関数とメソッドはコンパイルしない（known_bugs.md #23）。仮引数・ブロックや `for` の中で
/// 宣言したローカルへの代入はコンパイルする。⚠ 以前は大域への `=` を黙って出さず、`+=` は未定義の値（`%_UNDEF`）を
/// 読む IR になった（clang が落ちてモジュール全体がネイティブにならない）。
#[test]
fn functions_assigning_globals_are_left_to_the_interpreter() {
    let src = "\
mut counter = 0

fn set_c(let v: int) -> None:
    counter = v

fn bump() -> None:
    counter += 1

fn in_block(let v: int) -> None:
    if v > 0:
        counter = v

class Acc:
    mut n: int

    fn record(mut self) -> None:
        counter += 1
        self.n += 1

    fn local(mut self, let k: int) -> int:
        mut s = k
        s += self.n
        return s

fn locals(mut p: int, let n: int) -> int:
    p += 1
    mut acc = 0
    for i in range(n):
        mut sq = i * i
        sq += 1
        acc += sq
    if n > 0:
        mut t = 1
        t = t + acc
        acc = t
    return acc + p

fn read_only() -> int:
    return counter
";
    let tokens = Lexer::new(src, "").tokenize();
    let stmts = Parser::new(tokens, None).parse_program().expect("parse error");
    let (ir, exports) = generate_llvm_module(&stmts, &AstAnnotations::default()).expect("eligible functions");
    assert!(!ir.contains("%_UNDEF"), "undefined value in the IR");
    let names: Vec<&str> = exports.iter().map(|e| e.name.as_str()).collect();
    for compiled in ["locals", "read_only", "local"] {
        assert!(names.contains(&compiled), "{compiled} must be compiled: {names:?}");
    }
    for left in ["set_c", "bump", "in_block", "record"] {
        assert!(!names.contains(&left), "{left} must not be compiled: {names:?}");
    }
}

/// 値を積まない関数（`pushing_fns` に無い）を `int` / ハンドルを返す直接呼び出しで呼ぶときは、被演算子を複製して
/// いなければ巻き戻さない（copy_alloc_speed_plan.md 7-12・`float` の 9-2 と同じ）。`let` で受ける `mut` の値を
/// 渡す呼び出しは、保存点を複製しうる道でだけ取り、取ったときだけ巻き戻す（`CallSave::Lazy`）。積む関数は今までどおり
/// 必ず巻き戻す。⚠ 以前は `int` / ハンドルを返す呼び出しと、`let` で受ける `mut` の値を渡す呼び出しが、毎回
/// `ar_arena_save` / `ar_arena_compact` を払っていた。
#[test]
fn calls_of_functions_that_push_nothing_skip_the_arena_rewind() {
    let src = "\
class V:
    mut x: float
    mut tag: str

    fn reset(mut self) -> None:
        self.x = 0.0

fn twice(let n: int) -> int:
    return n * 2

fn same(mut v: V) -> V:
    return v

fn label(let n: int) -> int:
    return len(str(n))

fn peek(let v: V) -> float:
    return v.x

fn grab(let v: V) -> str:
    return v.tag

fn drive(mut v: V, let n: int) -> int:
    mut acc = 0
    mut i = 0
    while i < n:
        acc += twice(i)
        v.reset()
        mut w = same(v)
        acc += label(i)
        i += 1
    return acc

fn copies(mut v: V) -> str:
    let x = peek(v)
    return grab(v)
";
    let tokens = Lexer::new(src, "").tokenize();
    let stmts = Parser::new(tokens, None).parse_program().expect("parse error");
    let (ir, _) = generate_llvm_module(&stmts, &AstAnnotations::default()).expect("eligible functions");
    // 呼ぶ側の `_impl` の本体だけを見る（`_tl` の包みも `_impl` を呼ぶが、境界なので巻き戻さない）
    let mut lines: Vec<&str> = Vec::new();
    let mut inside = false;
    for l in ir.lines() {
        if l.starts_with("define ") {
            inside = l.contains("@drive_impl(") || l.contains("@copies_impl(");
        }
        if inside {
            lines.push(l);
        }
    }
    let compact_field = format!("i32 0, i32 {}", super::CB_ARENA_COMPACT);
    let is_compact = |l: &str| l.contains("getelementptr inbounds %ArCallbacks") && l.ends_with(&compact_field);
    #[derive(Debug, PartialEq)]
    enum Rewind { None, Always, IfSaved }
    // 呼び出しの直後に巻き戻しがあるか: コールバックを引く 2 行の中なら必ず、保存点の置き場が -1 でないかを
    // 比べて分かれた先（8 行の中）なら取ったときだけ
    let rewinds_after = |callee: &str| -> Vec<Rewind> {
        let call = format!("@{callee}_impl(");
        lines.iter().enumerate()
            .filter(|(_, l)| l.contains(&call) && l.contains(" call "))
            .map(|(i, _)| {
                let near = &lines[i + 1..(i + 3).min(lines.len())];
                let after = &lines[i + 1..(i + 9).min(lines.len())];
                if near.iter().any(|l| is_compact(l)) {
                    Rewind::Always
                } else if after.iter().any(|l| l.contains("icmp ne i64") && l.ends_with(", -1"))
                    && after.iter().any(|l| is_compact(l))
                {
                    Rewind::IfSaved
                } else {
                    Rewind::None
                }
            })
            .collect()
    };
    for (callee, want) in [
        ("twice", Rewind::None),
        ("V__reset", Rewind::None),
        ("same", Rewind::None),
        ("label", Rewind::Always),
        ("grab", Rewind::Always),
        ("peek", Rewind::IfSaved),
    ] {
        let r = rewinds_after(callee);
        assert!(!r.is_empty(), "{callee}_impl is called directly");
        assert!(r.iter().all(|x| *x == want), "a call of {callee} must rewind {want:?}: {r:?}");
    }
}

/// 覚えた番地（8-5）は、番地を取るコールバックと LLVM の組み込みのほかの `call` の直後で捨てる（known_bugs.md #26）。
/// ⚠ 以前は変数へ書いたときだけ捨てていたので、誤りを積んだコールバックの後の書き込みが、番地を取り直す
/// コールバック（先に積まれた誤りがあれば空を返す）を通らずに番地へ書いた。
#[test]
fn remembered_blocks_are_dropped_after_other_calls() {
    use super::context::{cb_field_of_call, CallKind};
    let mut regs = std::collections::HashMap::new();
    let gep = |n: usize, r: &str| format!("%g{r} = getelementptr inbounds %ArCallbacks, ptr %cb, i32 0, i32 {n}");
    let load = |r: &str| format!("%f{r} = load ptr, ptr %g{r}");
    for (n, r) in [(super::CB_INST_BLOCK, "a"), (super::CB_ATTR_BLOCK, "b"), (14, "c")] {
        assert_eq!(cb_field_of_call(&gep(n, r), &mut regs), None);
        assert_eq!(cb_field_of_call(&load(r), &mut regs), None);
    }
    let classify = |line: &str, regs: &mut std::collections::HashMap<String, usize>| cb_field_of_call(line, regs);
    assert_eq!(classify("%p = call ptr (i64, ptr, ptr) %fa(i64 %h, ptr @_dcd0, ptr @_ic1)", &mut regs), Some(CallKind::KeepsBlocks));
    assert_eq!(classify("%p = call ptr (i64, ptr, i32, ptr, ptr, ptr) %fb(i64 %h, ptr @_s1)", &mut regs), Some(CallKind::KeepsBlocks));
    assert_eq!(classify("%v = call i64 (i64, i64) %fc(i64 %h, i64 %i)", &mut regs), Some(CallKind::Other));
    assert_eq!(classify("%v = call double @llvm.sqrt.f64(double %x)", &mut regs), Some(CallKind::KeepsBlocks));
    assert_eq!(classify("%v = call i64 @step_impl(i64 %h)", &mut regs), Some(CallKind::Other));
    assert_eq!(classify("call void %unknown(i64 %h)", &mut regs), Some(CallKind::Other));
    assert_eq!(classify("store ptr null, ptr %_ip_p", &mut regs), None);

    let src = "\
class P:
    mut x: float
    mut y: float

fn after(mut p: P, let xs: list[int]) -> None:
    p.y = 1.0
    let k = xs[5]
    p.y = 3.0
";
    let tokens = Lexer::new(src, "").tokenize();
    let stmts = Parser::new(tokens, None).parse_program().expect("parse error");
    let (ir, _) = generate_llvm_module(&stmts, &AstAnnotations::default()).expect("eligible functions");
    let start = ir.find("@after_impl(").expect("after_impl is defined");
    let body: Vec<&str> = ir[start..].lines().take_while(|l| *l != "}").collect();
    let mut regs = std::collections::HashMap::new();
    let (mut kept, mut dropped) = (0, 0);
    for (i, line) in body.iter().enumerate() {
        let next = body.get(i + 1).copied().unwrap_or("");
        match cb_field_of_call(line.trim_start(), &mut regs) {
            Some(CallKind::KeepsBlocks) => {
                assert!(!next.contains("store ptr null, ptr %_ip_p"), "kept after {line}");
                kept += 1;
            }
            Some(CallKind::Other) => {
                assert_eq!(next.trim_start(), "store ptr null, ptr %_ip_p", "dropped right after {line}");
                dropped += 1;
            }
            None => {}
        }
    }
    assert!(kept >= 1, "the block of p is taken by CB_INST_BLOCK:\n{}", body.join("\n"));
    assert!(dropped >= 2, "the subscript and the slow paths drop it:\n{}", body.join("\n"));
}

/// 入れ子の受け手 `h.v` の番地は（変数, 属性）ごとに 1 つの置き場に覚え、置き場が空のときだけ `CB_ATTR_BLOCK` を呼ぶ
/// （copy_alloc_speed_plan.md 7-13）。親の変数へ書いたときと、番地を取るコールバックのほかの呼び出しの後で捨てる。
/// ⚠ 以前は `h.v.x` を読むたびに `CB_ATTR_BLOCK` を呼んでいた（`Body.bounce` は 1 回の呼び出しで 6 回）。
#[test]
fn nested_receiver_blocks_are_remembered_per_variable_and_field() {
    use super::context::{cb_field_of_call, CallKind};
    let src = "\
class V:
    mut x: float
    mut y: float

class H:
    mut v: V
    mut w: V
    mut tag: str

fn reads(let h: H) -> float:
    return h.v.x + h.v.y + h.v.x + h.w.x

fn rebinds(mut h: H, mut k: H) -> float:
    let a = h.v.x
    h = k
    return a + h.v.x
";
    let tokens = Lexer::new(src, "").tokenize();
    let mut stmts = Parser::new(tokens, None).parse_program().expect("parse error");
    // 入れ子の受け手の直接の道は、`h.v.x` が `float` だという型検査の注釈を要る（`--compile` と同じ入口）
    let (errors, _, annotations) = crate::interpreter::resolver::resolve_and_annotate(&mut stmts);
    assert!(errors.is_empty(), "{errors:?}");
    let (ir, _) = generate_llvm_module(&stmts, &annotations).expect("eligible functions");
    let body_of = |f: &str| -> Vec<String> {
        let start = ir.find(&format!("@{f}_impl(")).unwrap_or_else(|| panic!("{f}_impl is defined"));
        ir[start..].lines().take_while(|l| *l != "}").map(str::to_string).collect()
    };
    // `CB_ATTR_BLOCK` の呼び出しごとに、結果を書いた置き場（`store ptr %r, ptr %_apN`）
    let attr_block_slots = |body: &[String]| -> Vec<String> {
        let mut regs = std::collections::HashMap::new();
        let mut slots = Vec::new();
        for (i, line) in body.iter().enumerate() {
            let l = line.trim_start();
            if cb_field_of_call(l, &mut regs) == Some(CallKind::KeepsBlocks) && l.contains(" = call ptr (i64, ptr, i32,") {
                let dst = l.split(" = ").next().unwrap();
                let next = body[i + 1].trim_start();
                let slot = next
                    .strip_prefix(&format!("store ptr {dst}, ptr "))
                    .unwrap_or_else(|| panic!("the block of {l} is remembered: {next}"));
                slots.push(slot.to_string());
            }
        }
        slots
    };
    let reads = body_of("reads");
    let slots = attr_block_slots(&reads);
    assert_eq!(slots.len(), 4, "one fetch per read site:\n{}", reads.join("\n"));
    let distinct: std::collections::BTreeSet<&String> = slots.iter().collect();
    assert_eq!(distinct.len(), 2, "h.v and h.w each have one slot: {slots:?}");
    assert!(slots.iter().all(|s| s.starts_with("%_ap")), "{slots:?}");
    // 読み出しの前は置き場を見て、空のときだけ呼ぶ
    let v_slot = &slots[0];
    let loads = reads.iter().filter(|l| l.trim_start().contains(&format!("= load ptr, ptr {v_slot}"))).count();
    assert!(loads >= 3 * 2, "every read of h.v loads the slot (before and after the fetch): {loads}");

    let rebinds = body_of("rebinds");
    let slots = attr_block_slots(&rebinds);
    assert!(!slots.is_empty(), "h.v is remembered:\n{}", rebinds.join("\n"));
    let slot = &slots[0];
    let store_h = rebinds
        .iter()
        .position(|l| l.trim_start().starts_with("store i64 ") && l.ends_with(", ptr %_al_h"))
        .and_then(|first| {
            rebinds.iter().enumerate().skip(first + 1).find(|(_, l)| {
                l.trim_start().starts_with("store i64 ") && l.ends_with(", ptr %_al_h")
            })
        })
        .map(|(i, _)| i)
        .expect("h = k stores into h");
    assert_eq!(rebinds[store_h + 1].trim_start(), format!("store ptr null, ptr {slot}"), "h = k drops h.v");
}

/// raw レイアウトのクラスのメソッドは、番地で呼ぶ変種（`@X_blk`）を持つ（copy_alloc_speed_plan.md 7-14）。`mut self` は
/// 番地、`let self` と `let` のクラスの仮引数は数値のフィールドの生値で受け、本体は何も呼ばない。コールバックを使う
/// メソッドには作らない。呼ぶ側は、番地が空でない・全フィールドの初期化の印・`mut self` なら不変の印・ガード付きの
/// 受け手なら `class_id` を確かめてから呼び、外れたら今までの呼び出し（`_impl`）。
/// ⚠ 以前は `h.v.bump(1.0)` が `h.v` をアリーナに積み、ガード・文脈の切り替え・呼ばれた側の番地の取得を通っていた。
#[test]
fn methods_of_raw_classes_get_a_block_variant() {
    use crate::interpreter::value::block;
    let src = "\
class V:
    mut x: float
    mut y: float

    fn bump(mut self, let k: float) -> None:
        self.x = self.x + k

    fn dot(let self, let o: V) -> float:
        return self.x * o.x + self.y * o.y

    fn loud(let self) -> float:
        print(\"x\")
        return self.x

class H:
    mut v: V
    mut w: V
    mut name: str

fn drive(mut h: H) -> float:
    h.v.bump(1.0)
    let a = h.v.dot(h.w)
    let b = h.v.loud()
    return a + b
";
    let tokens = Lexer::new(src, "").tokenize();
    let mut stmts = Parser::new(tokens, None).parse_program().expect("parse error");
    let (errors, _, annotations) = crate::interpreter::resolver::resolve_and_annotate(&mut stmts);
    assert!(errors.is_empty(), "{errors:?}");
    let (ir, _) = generate_llvm_module(&stmts, &annotations).expect("eligible functions");
    let body_of = |head: &str| -> Option<Vec<String>> {
        let start = ir.find(head)?;
        Some(ir[start..].lines().take_while(|l| *l != "}").map(str::to_string).collect())
    };
    // 呼ばれる側: 並びのとおりの変種があり、本体は何も呼ばない
    let bump = body_of("define internal i64 @V__bump_blk(ptr %_bp_self, double %_fph1)").expect("bump has a block variant");
    let dot = body_of("define internal double @V__dot_blk(double %_fp_self__x, double %_fp_self__y, double %_fp_o__x, double %_fp_o__y)")
        .expect("dot has a block variant");
    for b in [&bump, &dot] {
        assert!(!b.iter().any(|l| l.contains(" call ")), "a block variant calls nothing:\n{}", b.join("\n"));
    }
    // `self.x = ..` は番地の `x` の位置（先頭の語の後）へそのまま書く
    let x_at = format!("getelementptr inbounds i8, ptr %_bp_self, i64 {}", block::FIELDS_OFFSET);
    assert!(bump.iter().any(|l| l.contains(&x_at)), "bump writes x at the block:\n{}", bump.join("\n"));
    assert!(bump.iter().any(|l| l.trim_start().starts_with("store double")), "{}", bump.join("\n"));
    assert!(!ir.contains("@V__loud_blk"), "loud uses a callback, so it has no block variant");

    // 呼ぶ側: 速い道で `_blk` を呼び、遅い道に今までの `_impl` を残す
    let drive = body_of("@drive_impl(").expect("drive is compiled").join("\n");
    assert!(drive.contains("call i64 @V__bump_blk(ptr "), "{drive}");
    assert!(drive.contains("call double @V__dot_blk(double "), "{drive}");
    assert!(drive.contains("@V__bump_impl(") && drive.contains("@V__dot_impl("), "the slow paths remain:\n{drive}");
    // 検査: 全フィールドの初期化の印（2 つ）・`mut self` は不変の印も・ガード付きの受け手の `class_id`
    let init = (block::init_bit(0) | block::init_bit(1)) as i64;
    let with_imm = (block::init_bit(0) | block::init_bit(1) | block::IMMUTABLE) as i64;
    assert!(drive.contains(&format!(", {with_imm}\n")), "bump checks the immutable flag:\n{drive}");
    assert!(drive.contains(&format!(", {init}\n")), "dot checks the init bits:\n{drive}");
    assert!(drive.contains(&format!(", {}\n", block::CLASS_ID_MASK as i64)), "the guarded receiver's class id is compared:\n{drive}");
}

/// `for` の周の終わりに残す値が無いループは、アリーナの巻き戻しを次の周の頭の取り出し（`CB_ITER_NEXT_AT`）にまとめる
/// （copy_alloc_speed_plan.md 7-15）: ループに入る所で見出しの置き場を -1 にしてから保存点を書き、戻り口では何も呼ばない。
/// 残す値（ハンドルのローカル）があるループは、今までどおり戻り口で `CB_COMPACT_MANY`（置き場は -1 のまま）。
/// ⚠ 以前は 1 周に `CB_ITER_NEXT` と `CB_COMPACT_MANY` の 2 回のコールバックを呼んでいた。
#[test]
fn for_loops_rewind_at_the_next_element_when_nothing_is_kept() {
    let src = "\
class C:
    mut n: int
    mut tag: str

fn total(let cs: list[C]) -> int:
    mut s = 0
    for c in cs:
        s += len(c.tag)
    return s

fn keep(let cs: list[C]) -> str:
    mut seen = \"\"
    for c in cs:
        seen = c.tag
    return seen
";
    let tokens = Lexer::new(src, "").tokenize();
    let mut stmts = Parser::new(tokens, None).parse_program().expect("parse error");
    let (errors, _, annotations) = crate::interpreter::resolver::resolve_and_annotate(&mut stmts);
    assert!(errors.is_empty(), "{errors:?}");
    let (ir, _) = generate_llvm_module(&stmts, &annotations).expect("eligible functions");
    let body_of = |f: &str| -> String {
        let start = ir.find(&format!("@{f}_impl(")).unwrap_or_else(|| panic!("{f}_impl is defined"));
        ir[start..].lines().take_while(|l| *l != "}").collect::<Vec<_>>().join("\n")
    };
    let field = |n: usize| format!("i32 0, i32 {n}\n");
    let slot_of = |body: &str| -> String {
        let at = body.find("store i64 -1, ptr %_itsave").expect("the header's slot is reset before the loop");
        let rest = &body[at + "store i64 -1, ptr ".len()..];
        rest.split(|c: char| c == '\n').next().unwrap().to_string()
    };
    let stores_into = |body: &str, slot: &str| body.lines().filter(|l| l.trim_start().starts_with("store i64 ") && l.ends_with(&format!(", ptr {slot}"))).count();

    let total = body_of("total");
    assert!(total.contains(&field(super::CB_ITER_NEXT_AT)), "{total}");
    assert!(!total.contains(&field(super::CB_COMPACT_MANY)), "nothing is kept, so no rewind at the latch:\n{total}");
    let slot = slot_of(&total);
    assert_eq!(stores_into(&total, &slot), 2, "-1, then the save point taken when the loop starts:\n{total}");

    let keep = body_of("keep");
    assert!(keep.contains(&field(super::CB_ITER_NEXT_AT)), "{keep}");
    assert!(keep.contains(&field(super::CB_COMPACT_MANY)), "`seen` is kept at the latch:\n{keep}");
    let slot = slot_of(&keep);
    assert_eq!(stores_into(&keep, &slot), 1, "the header's slot stays -1:\n{keep}");
}

/// `for` はループに入るときに `CB_ITER_BEGIN` で要素のハンドルの並び（番地と個数）を受け取り、周ごとに並びを読む
/// （copy_alloc_speed_plan.md 7-20）。1 つずつ取る `CB_ITER_NEXT_AT` は個数が -1（集合・ジェネレータなど）の道だけ。
/// 本体が値を積み、周の終わりに残す値が無いループは、並びの道の見出しで保存点まで巻き戻す（`CB_ARENA_COMPACT(0, ..)`）。
/// 文の `for` も式の `for` も同じ。⚠ 以前は入口が `CB_ITER_FROM` で、1 周ごとに `CB_ITER_NEXT_AT` を呼んでいた。
#[test]
fn for_loops_read_the_handle_run_without_a_callback_per_round() {
    let src = "\
class C:
    mut n: int
    mut tag: str

fn count(let cs: list[C]) -> int:
    mut k = 0
    for c in cs:
        k += 1
    return k

fn total(let cs: list[C]) -> int:
    mut s = 0
    for c in cs:
        s += len(c.tag)
    return s

fn doubled(let xs: list[int]) -> list[int]:
    return for x in xs ->list[int]:
        loop_yield x * 2
";
    let tokens = Lexer::new(src, "").tokenize();
    let mut stmts = Parser::new(tokens, None).parse_program().expect("parse error");
    let (errors, _, annotations) = crate::interpreter::resolver::resolve_and_annotate(&mut stmts);
    assert!(errors.is_empty(), "{errors:?}");
    let (ir, _) = generate_llvm_module(&stmts, &annotations).expect("eligible functions");
    let body_of = |f: &str| -> String {
        let start = ir.find(&format!("@{f}_impl(")).unwrap_or_else(|| panic!("{f}_impl is defined"));
        ir[start..].lines().take_while(|l| *l != "}").collect::<Vec<_>>().join("\n")
    };
    let field = |n: usize| format!("i32 0, i32 {n}\n");
    for f in ["count", "total", "doubled"] {
        let body = body_of(f);
        assert_eq!(body.matches(&field(super::CB_ITER_BEGIN)).count(), 1, "{f}:\n{body}");
        assert!(!body.contains(&field(super::CB_ITER_FROM)), "{f}:\n{body}");
        assert_eq!(body.matches(&field(super::CB_ITER_NEXT_AT)).count(), 1, "only the one-at-a-time path:\n{body}");
        assert!(body.contains("= getelementptr inbounds i64, ptr "), "the round reads the run:\n{body}");
        assert!(body.contains("= alloca [2 x i64]"), "the run's address and count come back through memory:\n{body}");
    }
    // 本体が積まない: 並びの道の巻き戻しは出すが、置き場は -1 のまま（LLVM が畳む）
    let count = body_of("count");
    let slot = count.split("store i64 -1, ptr ").nth(1).and_then(|r| r.lines().next()).expect("the header's slot").to_string();
    let stores = count.lines().filter(|l| l.trim_start().starts_with("store i64 ") && l.ends_with(&format!(", ptr {slot}"))).count();
    assert_eq!(stores, 1, "{count}");
    // 本体が積み、残す値が無い: 保存点を置き場へ書き、並びの道の見出しで巻き戻す
    let total = body_of("total");
    let slot = total.split("store i64 -1, ptr ").nth(1).and_then(|r| r.lines().next()).expect("the header's slot").to_string();
    let stores = total.lines().filter(|l| l.trim_start().starts_with("store i64 ") && l.ends_with(&format!(", ptr {slot}"))).count();
    assert_eq!(stores, 2, "-1, then the save point:\n{total}");
    assert!(total.contains(&field(super::CB_ARENA_COMPACT)) && total.contains("(i64 0, i64 %"), "{total}");
}

/// raw レイアウトでないクラスの数値のフィールドの複合代入は、値の番地を返すコールバック（`CB_NUM_FIELD_PTR`）の 1 回で
/// 読んで演算して書く（copy_alloc_speed_plan.md 7-16）。外れたときの道に今までの読み出しと書き込みを残す。受け手が名前で
/// ないとき・raw レイアウトのクラス（番地から直接読み書きする道）には出さない。
/// ⚠ 以前は `self.hits += k` が `CB_GET_INT_FIELD_IC` と `CB_ASSIGN_INT_FIELD_IC` の 2 回だった。
#[test]
fn compound_assignments_of_numeric_fields_take_one_callback() {
    let src = "\
class C:
    mut name: str
    mut hits: int
    mut mass: float

    fn tick(mut self, let k: int) -> None:
        self.hits += k
        self.mass += 0.5

class R:
    mut n: int

fn on_local(mut c: C) -> None:
    c.hits *= 2

fn on_raw(mut r: R) -> None:
    r.n += 1

fn on_element(mut cs: list[C]) -> None:
    cs[0].hits += 1
";
    let tokens = Lexer::new(src, "").tokenize();
    let mut stmts = Parser::new(tokens, None).parse_program().expect("parse error");
    let (errors, _, annotations) = crate::interpreter::resolver::resolve_and_annotate(&mut stmts);
    assert!(errors.is_empty(), "{errors:?}");
    let (ir, _) = generate_llvm_module(&stmts, &annotations).expect("eligible functions");
    let body_of = |f: &str| -> String {
        let start = ir.find(&format!("@{f}_impl(")).unwrap_or_else(|| panic!("{f}_impl is defined"));
        ir[start..].lines().take_while(|l| *l != "}").collect::<Vec<_>>().join("\n")
    };
    let field = |n: usize| format!("i32 0, i32 {n}\n");
    let count = |body: &str, n: usize| body.matches(&field(n)).count();

    let tick = body_of("C__tick");
    assert_eq!(count(&tick, super::CB_NUM_FIELD_PTR), 2, "one callback for each compound assignment:\n{tick}");
    assert!(tick.contains("store i64 ") && tick.contains("store double "), "the fast paths write through the address:\n{tick}");
    // 外れたときの道: 今までの読み出しと書き込み
    assert_eq!(count(&tick, super::CB_GET_INT_FIELD_IC), 1, "{tick}");
    assert_eq!(count(&tick, super::CB_ASSIGN_INT_FIELD_IC), 1, "{tick}");
    assert_eq!(count(&tick, super::CB_GET_FLOAT_FIELD_IC), 1, "{tick}");
    assert_eq!(count(&tick, super::CB_ASSIGN_FLOAT_FIELD_IC), 1, "{tick}");
    assert_eq!(count(&body_of("on_local"), super::CB_NUM_FIELD_PTR), 1, "a local receiver too");
    assert_eq!(count(&body_of("on_raw"), super::CB_NUM_FIELD_PTR), 0, "a raw-layout class reads and writes its block");
    assert_eq!(count(&body_of("on_element"), super::CB_NUM_FIELD_PTR), 0, "the receiver is not a name");
}

/// 値を積む箇所が遅い道だけにある関数は「値を積まない関数」になり、呼ぶ側が保存点と巻き戻しで囲まない
/// （copy_alloc_speed_plan.md 7-17）。①遅い道（入れ子の受け手の読み出しの外れ・番地で呼ぶ変種の外れ・ガードが外れた
/// 名前による呼び出しで結果を使わないもの）は、道の中で保存点を取って巻き戻す。②`a.b.x = <数の式>` で式が確かめられる
/// 番地だけを読むなら、書き込み先と読み出し先の番地を先に確かめて検査なしで読み書きする（`a.b` を積まない）。
/// 結果のハンドルを返す関数・確かめられない式を書く関数は、今までどおり値を積む。
/// ⚠ 以前は `self.vel.x = 0.0 - self.vel.x`（`Body.bounce`）が `self.vel` を積み、`step` → `bounce` と `step_all` → `step` の
///   呼び出しが毎回保存点と巻き戻しで囲まれた。
#[test]
fn slow_paths_rewind_so_their_functions_do_not_push() {
    use crate::interpreter::value::block;
    let src = "\
class V:
    mut x: float
    mut y: float

    fn dot(let self, let o: V) -> float:
        return self.x * o.x + self.y * o.y

class K:
    mut n: int
    mut tag: str

    fn bump(mut self) -> None:
        self.n += 1

    fn label(let self) -> str:
        return self.tag

class H:
    mut v: V
    mut w: V
    mut k: K
    mut name: str

fn reads(mut h: H) -> float:
    return h.v.x * 2.0

fn dots(mut h: H) -> float:
    return h.v.dot(h.w)

fn bumps(mut k: K) -> None:
    k.bump()

fn flips(mut h: H) -> None:
    h.v.x = 0.0 - h.v.x
    h.v.y = h.w.x * 2.0 + h.v.y

fn flips_named(mut h: H) -> None:
    h.v.x = 0.0 - h.v.x + float(len(h.name))

fn labels(mut k: K) -> str:
    let s = k.label()
    return s

fn call_reads(mut h: H) -> float:
    return reads(h)

fn call_dots(mut h: H) -> float:
    return dots(h)

fn call_bumps(mut k: K) -> None:
    bumps(k)

fn call_flips(mut h: H) -> None:
    flips(h)

fn call_flips_named(mut h: H) -> None:
    flips_named(h)

fn call_labels(mut k: K) -> int:
    return len(labels(k))
";
    let tokens = Lexer::new(src, "").tokenize();
    let mut stmts = Parser::new(tokens, None).parse_program().expect("parse error");
    let (errors, _, annotations) = crate::interpreter::resolver::resolve_and_annotate(&mut stmts);
    assert!(errors.is_empty(), "{errors:?}");
    let (ir, _) = generate_llvm_module(&stmts, &annotations).expect("eligible functions");
    let body_of = |f: &str| -> String {
        let start = ir.find(&format!("@{f}_impl(")).unwrap_or_else(|| panic!("{f}_impl is defined"));
        ir[start..].lines().take_while(|l| *l != "}").collect::<Vec<_>>().join("\n")
    };
    let field = |n: usize| format!("i32 0, i32 {n}\n");
    let saves = |f: &str| body_of(f).matches(&field(super::CB_ARENA_SAVE)).count();

    // 遅い道だけが積む関数: 呼ぶ側は囲まない
    for f in ["reads", "dots", "flips"] {
        assert_eq!(saves(&format!("call_{f}")), 0, "{f} does not push:\n{}", body_of(&format!("call_{f}")));
        assert!(saves(f) > 0, "{f} rewinds inside its slow paths:\n{}", body_of(f));
    }
    // 型を宣言した仮引数の受け手はガードなしで直接呼ぶ（遅い道が無い）: 積まないまま
    assert_eq!(saves("call_bumps"), 0, "{}", body_of("call_bumps"));
    // 結果のハンドルを返す・確かめられない式を書く: 今までどおり囲む
    assert_eq!(saves("call_labels"), 1, "labels pushes its result:\n{}", body_of("call_labels"));
    assert_eq!(saves("call_flips_named"), 1, "flips_named pushes h.v:\n{}", body_of("call_flips_named"));

    // ② 速い道: 書き込み先（不変の印・読む x の初期化の印）と読み出し先（y の初期化の印は書き込み先と同じ番地）を確かめる
    let flips = body_of("flips");
    let x_mask = (block::IMMUTABLE | block::init_bit(0)) as i64;
    let xy_mask = (block::IMMUTABLE | block::init_bit(1)) as i64;
    assert!(flips.contains(&format!(", {x_mask}\n")), "h.v.x = 0.0 - h.v.x checks x's init bit and the flag:\n{flips}");
    assert!(flips.contains(&format!(", {xy_mask}\n")), "h.v.y = .. + h.v.y checks y's init bit and the flag:\n{flips}");
    assert!(flips.contains(&format!(", {}\n", block::init_bit(0) as i64)), "h.w.x is checked for x's init bit:\n{flips}");
    assert!(flips.contains("fsub double 0.0, ") && flips.contains("store double "), "{flips}");
}

/// 同じ親の変数の入れ子の受け手を 2 つ以上取る箇所（番地で呼ぶ変種の受け手と引数・確かめてから書く代入）は、
/// `CB_ATTR_BLOCKS` の 1 回で番地をまとめて取る（copy_alloc_speed_plan.md 7-18）。件ごとの表は DLL の定数。
/// 同じ属性を 2 度使う・親が違うときはまとめない。
/// ⚠ 以前は `self.pos.add_scaled(self.vel, dt)` が `CB_ATTR_BLOCK` を 2 回呼んでいた（親を 2 回たどる）。
#[test]
fn nested_receivers_of_one_parent_are_fetched_together() {
    let src = "\
class V:
    mut x: float
    mut y: float

    fn add(mut self, let o: V) -> None:
        self.x = self.x + o.x
        self.y = self.y + o.y

class H:
    mut v: V
    mut w: V
    mut u: V
    mut name: str

fn adds(mut h: H) -> None:
    h.v.add(h.w)

fn adds_itself(mut h: H) -> None:
    h.v.add(h.v)

fn adds_other(mut a: H, mut b: H) -> None:
    a.v.add(b.w)

fn sums(mut h: H) -> None:
    h.v.x = h.w.x + h.u.y
";
    let tokens = Lexer::new(src, "").tokenize();
    let mut stmts = Parser::new(tokens, None).parse_program().expect("parse error");
    let (errors, _, annotations) = crate::interpreter::resolver::resolve_and_annotate(&mut stmts);
    assert!(errors.is_empty(), "{errors:?}");
    let (ir, _) = generate_llvm_module(&stmts, &annotations).expect("eligible functions");
    let body_of = |f: &str| -> String {
        let start = ir.find(&format!("@{f}_impl(")).unwrap_or_else(|| panic!("{f}_impl is defined"));
        ir[start..].lines().take_while(|l| *l != "}").collect::<Vec<_>>().join("\n")
    };
    let field = |n: usize| format!("i32 0, i32 {n}\n");
    let count = |f: &str, n: usize| body_of(f).matches(&field(n)).count();

    assert_eq!(count("adds", super::CB_ATTR_BLOCKS), 1, "{}", body_of("adds"));
    assert_eq!(count("adds", super::CB_ATTR_BLOCK), 0, "h.v and h.w come from one call:\n{}", body_of("adds"));
    assert!(body_of("adds").contains("call void (i64, i32, ptr, ptr) ") && body_of("adds").contains(", i32 2, ptr @_ab"));
    assert_eq!(count("sums", super::CB_ATTR_BLOCKS), 1, "{}", body_of("sums"));
    assert!(body_of("sums").contains(", i32 3, ptr @_ab"), "h.v, h.w and h.u together:\n{}", body_of("sums"));
    // 件ごとの表: (名前, 長さ, IC のセル, クラスの表, クラスのセル)
    assert!(ir.lines().any(|l| l.starts_with("@_ab") && l.contains("= private constant [3 x { ptr, i64, ptr, ptr, ptr }]")));
    // 同じ属性を 2 度・親が違う: まとめない
    assert_eq!(count("adds_itself", super::CB_ATTR_BLOCKS), 0, "{}", body_of("adds_itself"));
    assert_eq!(count("adds_other", super::CB_ATTR_BLOCKS), 0, "{}", body_of("adds_other"));
    assert_eq!(count("adds_other", super::CB_ATTR_BLOCK), 2, "{}", body_of("adds_other"));
}

/// `self` の入れ子の受け手の番地を覚える `mut self` のメソッドは、番地を受け取る変種（`@X_pre(.., ptr ..)`）を持ち、
/// 置き場をその番地で始める（copy_alloc_speed_plan.md 7-19）。`_impl` は空の番地で `_pre` を呼ぶだけ。同じクラスの
/// `self.m(..)` は覚えている番地を渡して `_pre` を呼び、受け手が `self` でない呼び出しは `_impl`。`let self` のメソッドには
/// 作らない。番地で呼ぶ変種（`_blk`）の呼び出しの後は覚えた番地を捨てない。
/// ⚠ 以前は `step` が `self.pos` / `self.vel` の番地を持っていても、`Body.bounce` が入口で空にして取り直していた。
#[test]
fn methods_called_on_self_receive_the_remembered_blocks() {
    let src = "\
class V:
    mut x: float
    mut y: float

    fn bump(mut self, let k: float) -> None:
        self.x = self.x + k

class K:
    mut v: V
    mut w: V
    mut name: str

    fn flip(mut self) -> None:
        self.v.x = 0.0 - self.v.x
        self.w.y = self.w.y + self.v.x

    fn look(let self) -> float:
        return self.v.x

    fn step(mut self) -> None:
        self.v.y = self.v.y + self.w.x
        self.v.bump(1.0)
        self.flip()

    fn other(mut self, mut o: K) -> None:
        self.v.y = self.v.y + 1.0
        o.flip()
";
    let tokens = Lexer::new(src, "").tokenize();
    let mut stmts = Parser::new(tokens, None).parse_program().expect("parse error");
    let (errors, _, annotations) = crate::interpreter::resolver::resolve_and_annotate(&mut stmts);
    assert!(errors.is_empty(), "{errors:?}");
    let (ir, _) = generate_llvm_module(&stmts, &annotations).expect("eligible functions");
    let def = |head: &str| -> String {
        let start = ir.find(head).unwrap_or_else(|| panic!("{head} is defined:\n{ir}"));
        ir[start..].lines().take_while(|l| *l != "}").collect::<Vec<_>>().join("\n")
    };
    let flip = def("define internal i64 @K__flip_pre(i64 %_h0, ptr %_pb0, ptr %_pb1)");
    assert!(flip.contains("store ptr %_pb0, ptr %_ap") && flip.contains("store ptr %_pb1, ptr %_ap"), "{flip}");
    let flip_impl = def("define internal i64 @K__flip_impl(i64 %_h0)");
    assert!(flip_impl.contains("call i64 @K__flip_pre(i64 %_h0, ptr null, ptr null)"), "{flip_impl}");
    assert!(!ir.contains("@K__look_pre"), "a `let self` method gets no block-receiving variant");
    let step = def("define internal i64 @K__step_pre(");
    let call = step.lines().find(|l| l.contains("call i64 @K__flip_pre(")).unwrap_or_else(|| panic!("{step}"));
    assert!(!call.contains("ptr null"), "step hands over its remembered blocks: {call}");
    // `self.v.bump(..)`（番地で呼ぶ変種）の後も、覚えた番地はそのまま（空にしない）
    let blk_at = step.lines().position(|l| l.contains("call i64 @V__bump_blk(")).unwrap_or_else(|| panic!("{step}"));
    let next = step.lines().nth(blk_at + 1).unwrap_or("");
    assert!(!next.contains("store ptr null"), "the blocks survive a `_blk` call:\n{step}");
    let other = def("define internal i64 @K__other_pre(");
    assert!(other.contains("call i64 @K__flip_impl("), "o is not self: no blocks to hand over:\n{other}");
}

/// 例外を出しうる関数（コールバックを呼ぶ関数・`raise` を持つ関数）を直接呼んだ後だけ、例外が積まれているかを
/// `CB_EXC_PENDING` で見て、積まれていればすぐ戻る（known_bugs.md #28）。コールバックを呼ばない数値だけの関数の
/// 呼び出しには何も足さない（不動点で決める・呼ぶ側を呼ばれる側より先に生成しても同じ）。`float` を返す関数の
/// `raise` は `ret double` で戻る（#27・以前は `ret i64 -2` で clang が落ちた）。
#[test]
fn direct_calls_check_pending_exceptions_only_after_failing_callees() {
    let src = "\
fn caller_pure(let n: int) -> int:
    return sq(n) + 1

fn caller_fail(let n: int) -> int:
    let k = fail_i(n)
    return k + 1

fn deep(let n: int) -> int:
    return caller_fail(n) + 1

fn sq(let x: int) -> int:
    return x * x

fn fail_i(let n: int) -> int:
    if n > 0:
        raise ValueError(\"boom\")
    return 0

fn fail_f(let n: int) -> float:
    if n > 0:
        raise ValueError(\"boom\")
    return 0.5
";
    let tokens = Lexer::new(src, "").tokenize();
    let mut stmts = Parser::new(tokens, None).parse_program().expect("parse error");
    let (errors, _, annotations) = crate::interpreter::resolver::resolve_and_annotate(&mut stmts);
    assert!(errors.is_empty(), "{errors:?}");
    let (ir, _) = generate_llvm_module(&stmts, &annotations).expect("eligible functions");
    let body_of = |f: &str| -> String {
        let start = ir.find(&format!("@{f}_impl(")).unwrap_or_else(|| panic!("{f}_impl is defined"));
        ir[start..].lines().take_while(|l| *l != "}").collect::<Vec<_>>().join("\n")
    };
    let pending = format!("i32 0, i32 {}\n", super::CB_EXC_PENDING);
    // 数値だけの関数の呼び出しには足さない
    let pure = body_of("caller_pure");
    assert!(pure.contains("@sq_impl("), "{pure}");
    assert!(!pure.contains(&pending), "{pure}");
    // raise する関数の呼び出しの後で見て、積まれていればすぐ戻る（呼ぶ側も例外を出しうるので、その呼ぶ側も見る）
    for f in ["caller_fail", "deep"] {
        let body = body_of(f);
        assert_eq!(body.matches(&pending).count(), 1, "{f}:\n{body}");
        assert!(body.contains("ret i64 -2"), "{f} returns at once:\n{body}");
    }
    // float を返す関数の raise は double で戻る
    let ff = body_of("fail_f");
    assert!(ff.contains("ret double 0.0"), "{ff}");
    assert!(!ff.contains("ret i64"), "{ff}");
}
