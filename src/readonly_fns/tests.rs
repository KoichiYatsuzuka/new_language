use super::analyze;
use crate::lexer::Lexer;
use crate::parser::Parser;

fn names(source: &str) -> Vec<String> {
    let tokens = Lexer::new(source, "").tokenize();
    let stmts = Parser::new(tokens, None).parse_program().expect("parse error");
    analyze(&stmts).sorted_names().into_iter().map(str::to_string).collect()
}

const CLASS_V: &str = "class V:\n    mut x: int\n";

/// 読むだけと判定するもの・しないものの表（copy_alloc_speed_plan.md 4-2）。
#[test]
fn verdicts() {
    let src = format!(
        "{CLASS_V}
    fn get(let self) -> int:
        return self.x
    fn add(let self, let o: V) -> int:
        return self.x + o.get()
    fn set(mut self, let v: int) -> None:
        self.x = v
    fn me(let self) -> V:
        return self
    fn same(let self, let o: V) -> bool:
        return self === o
    fn poke(let self, mut o: V) -> int:
        o.x = 1
        return self.x

fn total(let vs: list[V]) -> int:
    mut s = 0
    for v in vs:
        s += v.get()
    return s

fn grows(let vs: list[V]) -> int:
    mut ys = vs
    ys.append(V(1))
    return len(ys)

fn calls_value(let f: int, let v: V) -> int:
    return f(v)

mut counter = 0
fn bumps(let v: V) -> int:
    counter = counter + 1
    return v.x
"
    );
    assert_eq!(names(&src), ["add", "get", "total"]);
}

/// 自分の `mut` 仮引数だけを書き換える関数（4-6）。
#[test]
fn confined_writers() {
    let src = format!(
        "{CLASS_V}
    fn bump(mut self, let o: V) -> None:
        self.x = self.x + o.x
    fn leak(mut self, let o: V) -> V:
        self.x = o.x
        return o
    fn get(let self) -> int:
        return self.x

mut total = 0
fn tally(mut v: V) -> None:
    v.x = 0
    total = total + 1
"
    );
    let tokens = Lexer::new(&src, "").tokenize();
    let stmts = Parser::new(tokens, None).parse_program().expect("parse error");
    let r = analyze(&stmts);
    // パーサが合成した `__init__`（`self.x = x` だけ）も入る
    assert_eq!(r.sorted_confined(), ["__init__", "bump"]);
    assert!(r.is_confined_writer("get") && r.is_confined_writer("bump") && !r.is_confined_writer("tally"));
}

/// 同じ名前の定義が 1 つでも外れれば、その名前は入らない。
#[test]
fn same_name_is_closed_over_every_definition() {
    let src = "class A:\n    mut k: int\n    fn read(let self) -> int:\n        return self.k\n\
                   class B:\n    mut k: int\n    fn read(mut self) -> int:\n        self.k = 0\n        return self.k\n\
                   fn use_a(let a: A) -> int:\n    return a.read()\n";
    assert!(names(src).is_empty());
}

/// 読むだけでない特殊メソッドがあれば、どの関数も入れない（演算子が何を呼ぶかは名前から分からない）。
#[test]
fn impure_special_method_clears_everything() {
    let ok = format!("{CLASS_V}    fn get(let self) -> int:\n        return self.x\n    fn __str__(let self) -> str:\n        return str(self.x)\n");
    assert_eq!(names(&ok), ["__str__", "get"]);
    let bad = format!("{CLASS_V}    fn get(let self) -> int:\n        return self.x\n    fn __str__(mut self) -> str:\n        self.x = 0\n        return str(self.x)\n");
    assert!(names(&bad).is_empty());
}
