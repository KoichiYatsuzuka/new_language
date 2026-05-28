# auto_init_test.tl
# デフォルトコンストラクタ（auto-init）の動作確認
#
# ルール:
#   - 初期値なしの mut/let フィールドが存在 → auto-init を生成
#   - 初期値ありのフィールドは auto-init のパラメータに含まれない
#   - 同一シグネチャの __init__ が明示定義されている場合 → override（auto-init を生成しない）
#   - 異なるシグネチャの __init__ が定義されている場合 → overload（auto-init も存在する）

# ── 1. 必須フィールドのみ → auto-init(mut self, x, y) が生成される ────────
class Vec2:
    mut x: float
    mut y: float

let v = Vec2(1.5, 2.5)
print("=== 1. 必須フィールドのみ ===")
print("x:", v.x, " y:", v.y)

# ── 2. すべてのフィールドに初期値あり → auto-init は生成されない ────────────
class Defaults:
    mut a: int = 10
    let b: str = "hello"

let d = Defaults()
print("=== 2. すべてデフォルト値あり ===")
print("a:", d.a, " b:", d.b)

# ── 3. 同一シグネチャの __init__ → override（auto-init 生成なし） ─────────
class Override:
    mut x: int
    fn __init__(mut self, x: int) -> None:
        self.x = x * 2

let o = Override(5)
print("=== 3. 同一シグネチャで override ===")
print("x:", o.x)   # 5*2 = 10

# ── 4. 異なるシグネチャの __init__ → overload（auto-init も存在） ─────────
class Overload:
    mut x: int
    fn __init__(mut self, x: int, bonus: int) -> None:
        self.x = x + bonus

let ol1 = Overload(3)       # auto-init: x=3
let ol2 = Overload(3, 10)   # explicit: x=3+10=13
print("=== 4. 異なるシグネチャで overload ===")
print("ol1.x:", ol1.x, " ol2.x:", ol2.x)

# ── 5. let フィールド（初期値なし）→ コンストラクタで設定後はイミュータブル ──
class Immutable:
    let name: str

let im = Immutable("Alice")
print("=== 5. let フィールド（初期値なし） ===")
print("name:", im.name)

# ── 6. const フィールドは auto-init のパラメータに含まれない ─────────────
class WithConst:
    const MAX: int = 100
    mut value: int

let wc = WithConst(42)
print("=== 6. const フィールドは auto-init 対象外 ===")
print("value:", wc.value, " MAX:", wc.MAX)

# ── 7. 初期値なし + 初期値あり混在 → 初期値なしのみ auto-init に含まれる ──
class Mixed:
    mut required: int
    mut optional: int = 99

let m = Mixed(7)
print("=== 7. 必須フィールドと任意フィールド混在 ===")
print("required:", m.required, " optional:", m.optional)

print("=== All auto-init tests passed! ===")
