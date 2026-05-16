# type_value_test.tl
# `type` 型の動作確認

# ── 1. 組み込み型を変数に代入 ───────────────────────────────────────
let ti: type = int
let ts: type = str
let tf: type = float
let tb: type = bool

print("=== 1. 組み込み型を変数に代入 ===")
print(ti)
print(ts)
print(tf)
print(tb)

# ── 2. クラス型を type 変数に代入 ─────────────────────────────────
class Point:
    mut x: int
    mut y: int

let tp: type = Point

print("=== 2. クラス型を type 変数に代入 ===")
print(tp)

# ── 3. 型の等値比較 ───────────────────────────────────────────────
print("=== 3. 型の等値比較 ===")
print(ti == int)
print(ti == str)
print(ts == str)

# ── 4. 関数の引数として型を渡す ──────────────────────────────────
fn describe(t: type) -> str:
    return "got a type"

print("=== 4. 関数引数として型を渡す ===")
print(describe(int))
print(describe(float))
print(describe(Point))

# ── 5. リストに型を格納 ───────────────────────────────────────────
let types: list[type] = [int, str, float, bool]
print("=== 5. リストに型を格納 ===")
for t in types:
    print(t)

print("=== All type value tests passed! ===")
