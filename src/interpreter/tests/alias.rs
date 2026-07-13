// tests/alias.rs — alias（コンパイル時 AST 置換）の実行時挙動テスト。
//
// alias はパース時に右辺へ置換される純粋な構文別名。参照ごとに右辺が再評価され、
// lvalue（代入対象）としても透過的に振る舞う点が `let` と異なる。

use super::*;

/// 型への alias: コンストラクタ式として使える（int と完全に等価）。
#[test]
fn alias_to_type_as_constructor() {
    let v = run_get(
        "alias object_handle: int\nlet h = object_handle(42)\n",
        "h",
    );
    assert_int(v, 42);
}

/// lvalue への alias: 参照のたびに再評価され、代入対象としても透過する。
#[test]
fn alias_lvalue_read_and_write() {
    let src = concat!(
        "mut data_dict: dict[str, int] = {\"often_used_key\": 1}\n",
        "alias item: data_dict[\"often_used_key\"]\n",
        "item = 5\n",
        "item += 10\n",
        "let result = item\n",
    );
    // item は data_dict["often_used_key"] に展開されるため、書き込みが辞書に反映される。
    assert_int(run_get(src, "result"), 15);
}

/// lvalue への alias: 書き込みが元の辞書エントリを更新することを直接確認する。
#[test]
fn alias_lvalue_updates_underlying_dict() {
    let src = concat!(
        "mut data_dict: dict[str, int] = {\"k\": 1}\n",
        "alias item: data_dict[\"k\"]\n",
        "item = 99\n",
        "let back = data_dict[\"k\"]\n",
    );
    assert_int(run_get(src, "back"), 99);
}

/// テンプレート具体化への alias: 型としてもコンストラクタとしても使える。
#[test]
fn alias_to_template_instantiation() {
    let src = concat!(
        "trait Valued:\n",
        "    fn get(self) -> int:\n",
        "        ...\n",
        "class MyInt(Valued):\n",
        "    mut value: int\n",
        "    fn get(self) -> int:\n",
        "        return self.value\n",
        "class Box[T: Valued]:\n",
        "    mut item: T\n",
        "    fn unwrap(self) -> int:\n",
        "        return self.item.get()\n",
        "alias IntBox: Box[MyInt]\n",
        "let b: IntBox = IntBox(MyInt(7))\n",
        "let out = b.unwrap()\n",
    );
    assert_int(run_get(src, "out"), 7);
}

/// block 式への alias: 参照のたびにブロックが実行され、特殊化した関数値を返す。
#[test]
fn alias_to_block_expression_producing_function() {
    let src = concat!(
        "fn general_function(let a: int, let b: int) -> int:\n",
        "    return a + b\n",
        "alias sub_function: block->function:\n",
        "    fn specialized(let x: int) -> int:\n",
        "        return general_function(x, 3)\n",
        "    block_return specialized\n",
        "let f = sub_function\n",
        "let out = f(10)\n",
    );
    assert_int(run_get(src, "out"), 13);
}

/// alias はブロックスコープ: 関数内で宣言した alias は関数内でのみ有効。
#[test]
fn alias_is_block_scoped() {
    let src = concat!(
        "fn compute() -> int:\n",
        "    alias shortcut: 100 + 1\n",
        "    return shortcut\n",
        "let out = compute()\n",
    );
    assert_int(run_get(src, "out"), 101);
}

/// alias は再評価される（副作用が毎回起きる）ことを確認する。
#[test]
fn alias_is_reevaluated_each_use() {
    let src = concat!(
        "mut counter: list[int] = [0]\n",
        "fn bump() -> int:\n",
        "    counter[0] = counter[0] + 1\n",
        "    return counter[0]\n",
        "alias tick: bump()\n",
        "let a = tick\n",
        "let b = tick\n",
        "let total = a + b\n",
    );
    // tick は bump() に展開され、参照ごとに呼ばれる → a=1, b=2, total=3
    assert_int(run_get(src, "total"), 3);
}
