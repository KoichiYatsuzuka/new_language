# Union / Option のエラーサンプル
# 以下の 4 つの StaticTypeError が発生することを確認する

# ERROR 1: Union 値に算術演算子を適用
fn union_arithmetic(x: Union[int, str]) -> None:
    let y = x + 1

# ERROR 2: Union 値を特定型のパラメータへ渡す
fn needs_int(n: int) -> None:
    pass

fn union_to_typed_param(x: Union[int, str]) -> None:
    needs_int(x)

# ERROR 3: Option 値に比較演算子を適用
fn option_comparison(x: Option[int]) -> None:
    let z = x < 0

# ERROR 4: Option 値を特定型のパラメータへ渡す
fn option_to_typed_param(x: Option[int]) -> None:
    needs_int(x)
