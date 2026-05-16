# overload_errors.tl
# オーバーロードに存在しない引数の組み合わせを呼び出すサンプル。
# 型検査フェーズ（NoMatchingOverload）または実行時でエラーになるはず。

fn add(a: int, b: int) -> int:
    return a + b

fn add(a: str, b: str) -> str:
    return a + b

fn add(a: int, b: int, c: int) -> int:
    return a + b + c

# ── エラー 1: どのオーバーロードにも合わない引数の個数 ──────────
# add は 2引数か3引数だが、1引数で呼んでいる
add(1)

# ── エラー 2: どのオーバーロードにも合わない引数の個数（超過） ───
# add は最大3引数だが、4引数で呼んでいる
add(1, 2, 3, 4)
