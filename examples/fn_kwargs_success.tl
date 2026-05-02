# fn_kwargs_success.tl
# キーワード引数の構文・型検査テスト（全項目 OK のはず）
#
# fn 実行はまだ未実装なため、ユーザー定義関数の呼び出しはすべて
# 他の関数ボディ内に置き、トップレベルでは実行されないようにしている。
# 型検査フェーズはボディ内の呼び出しもチェックするので、
# キーワード引数の名前・型の検証はここで行われる。

# ── 1. 検査対象の関数定義 ────────────────────────────────────────

fn add(a: int, b: int) -> int:
    return a + b

fn greet(name: str, repeat: int) -> str:
    return name

fn compute(x: int, y: float, flag: bool) -> None:
    pass

# ── 2. 位置引数（従来どおり）────────────────────────────────────

fn test_positional() -> None:
    let r1 = add(10, 20)
    let r2 = greet("Alice", 3)

# ── 3. 全キーワード引数 ──────────────────────────────────────────

fn test_all_keyword() -> None:
    let r1 = add(a=1, b=2)
    let r2 = greet(name="Bob", repeat=5)
    compute(x=10, y=3.14, flag=True)

# ── 4. キーワード引数を逆順に指定 ────────────────────────────────

fn test_reversed_keyword() -> None:
    let r1 = add(b=99, a=1)
    let r2 = greet(repeat=2, name="Carol")

# ── 5. 位置引数とキーワード引数の混在 ────────────────────────────

fn test_mixed() -> None:
    let r1 = add(10, b=20)
    let r2 = greet("Dave", repeat=1)

# ── 6. 戻り値型の推論確認 ────────────────────────────────────────
# add は -> int なので戻り値を int 変数として扱い、順序比較できる。

fn test_return_type_inference() -> None:
    let result = add(a=3, b=4)
    if result < 100:
        pass

# ── トップレベル ─────────────────────────────────────────────────
# ユーザー定義関数は呼ばない（fn 実行未実装のため）。
# 型検査は上記すべてのボディを走査し、エラーがなければ exit(0)。

print("fn_kwargs_success: 型検査 OK")
print("  - 位置引数 / キーワード引数 / 逆順 / 混在 をすべて確認")
