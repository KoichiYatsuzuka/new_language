// type_check_tests/generators.rs — ジェネレータの要素型の静的検査（フェーズ10 10-7）。
//
// ⚠ 以前は `gen` の呼び出しの結果に型が付かず（`gen` の名前が `Unresolved` で宣言され、
//   シグネチャも登録されていなかった）、`for s in each(xs):` の `s` の型が分からなかった。
//   `let z: str = s` が静的にも実行時にも止まらなかった（実測）。

use super::*;

    const EACH: &str = concat!(
        "gen each(let items: list[int]) -> int:\n",
        "    for x in items:\n",
        "        yield x\n",
    );

    /// 最上位の `gen` の要素を別の型の変数へ入れると誤り（10-7 の再現）。
    #[test]
    fn gen_element_mismatch_err() {
        let src = format!("{EACH}for s in each([1, 2]):\n    let z: str = s\n    print(z)\n");
        assert!(err(&src));
    }

    /// 同じ型なら通る。
    #[test]
    fn gen_element_match_ok() {
        let src = format!("{EACH}for s in each([1, 2]):\n    let z: int = s\n    print(z)\n");
        assert!(ok(&src));
    }

    /// 呼び出しの結果は `generator[int]`（産出型ではない）。`int` の変数には入らない。
    #[test]
    fn gen_call_result_is_a_generator_err() {
        let src = format!("{EACH}let n: int = each([1])\n");
        assert!(err(&src));
    }

    /// `gen` の引数も検査する（以前は呼び出しの型が無く、引数も見ていなかった）。
    #[test]
    fn gen_argument_mismatch_err() {
        let src = format!("{EACH}for s in each([\"a\"]):\n    print(s)\n");
        assert!(err(&src));
    }

    /// 入れ子の `gen` も同じ。
    #[test]
    fn nested_gen_element_mismatch_err() {
        assert!(err(concat!(
            "fn outer() -> None:\n",
            "    gen inner() -> int:\n",
            "        yield 1\n",
            "    for v in inner():\n",
            "        let s: str = v\n",
            "        print(s)\n",
        )));
    }

    /// `gen` メソッドの要素型も伝わる（以前は素の `generator` で要素型が消えていた）。
    #[test]
    fn gen_method_element_mismatch_err() {
        assert!(err(concat!(
            "class Bag:\n",
            "    mut items: list[str]\n",
            "    gen each(self) -> str:\n",
            "        for it in self.items:\n",
            "            yield it\n",
            "let b = Bag([\"a\"])\n",
            "for it in b.each():\n",
            "    let n: int = it\n",
            "    print(n)\n",
        )));
    }

    /// 注釈 `generator[T]` を書ける。要素型が違えば誤り。
    #[test]
    fn generator_annotation_element_mismatch_err() {
        let src = format!("{EACH}fn f() -> generator[str]:\n    return each([1])\n");
        assert!(err(&src));
    }

    /// 要素型を書かない `generator` 注釈は、どの `generator[T]` も受ける（例題が使う形）。
    #[test]
    fn bare_generator_annotation_accepts_any_element_ok() {
        let src = format!(
            "{EACH}fn f() -> generator:\n    return each([1])\nfn g(let x: generator) -> None:\n    print(x.next())\ng(each([2]))\n"
        );
        assert!(ok(&src));
    }

    /// `generator[T]` 注釈を通した先でも要素型が伝わる。
    #[test]
    fn generator_annotation_flows_into_for_err() {
        let src = format!(
            "{EACH}fn f() -> generator[int]:\n    return each([1])\nfor v in f():\n    let s: str = v\n    print(s)\n"
        );
        assert!(err(&src));
    }

    /// `next()` の結果は要素の型（以前は型が無く、`let v: str = g.next()` が通っていた）。
    #[test]
    fn generator_next_element_mismatch_err() {
        let src = format!("{EACH}let g = each([1])\nlet v: str = g.next()\n");
        assert!(err(&src));
    }

    /// 同じ型なら通る。`close()` は `None`。
    #[test]
    fn generator_next_and_close_ok() {
        let src = format!("{EACH}let g = each([1])\nlet v: int = g.next()\ng.close()\n");
        assert!(ok(&src));
    }

    /// `gen` メソッドの本体の `self` もクラスの型（以前は `Unresolved` で何も検査していなかった）。
    #[test]
    fn gen_method_self_field_mismatch_err() {
        assert!(err(concat!(
            "class C:\n",
            "    mut n: int\n",
            "    gen items(self) -> int:\n",
            "        let s: str = self.n\n",
            "        yield 1\n",
        )));
    }

    /// 存在しないメンバーも止まる（以前は実行時の `AttributeError` まで気づかなかった）。
    #[test]
    fn gen_method_self_missing_member_err() {
        assert!(err(concat!(
            "class C:\n",
            "    mut n: int\n",
            "    gen items(self) -> int:\n",
            "        yield self.nope\n",
        )));
    }

    /// 正しい本体は通る。
    #[test]
    fn gen_method_self_ok() {
        assert!(ok(concat!(
            "class C:\n",
            "    mut n: int\n",
            "    gen items(self) -> int:\n",
            "        let k: int = self.n\n",
            "        yield k\n",
            "for v in C(1).items():\n",
            "    print(v)\n",
        )));
    }

    // --- `__iter__` の結果の型（python_builtins_plan.md のタスク 1-4）---

    /// 辞書はキー、list は要素、ジェネレータは自分自身の `generator[T]`。
    #[test]
    fn dunder_iter_types_ok() {
        assert!(ok(concat!(
            "let d: dict[str, int] = {\"a\": 1}\n",
            "let k: generator[str] = d.__iter__()\n",
            "let xs: list[int] = [1]\n",
            "let e: generator[int] = xs.__iter__()\n",
            "gen nums() -> int:\n",
            "    yield 1\n",
            "let g: generator[int] = nums().__iter__()\n",
        )));
    }

    /// 要素の型が違えば誤り（以前は `__iter__` の結果に型が無く素通りしていた）。
    #[test]
    fn dunder_iter_types_err() {
        assert!(err("let d: dict[str, int] = {\"a\": 1}\nlet k: generator[int] = d.__iter__()\n"));
    }
