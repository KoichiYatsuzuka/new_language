# ============================================================
# test_lang showcase — 実装済み機能の動作確認
# ============================================================

# --- 1. 変数宣言 ---
let lang    = "test_lang"
mut version = 1
const BUILD = 2026

print("=== 1. 変数宣言 ===")
print(lang, version, BUILD)

version += 1
print("version after increment:", version)

# --- 2. 算術・文字列演算 ---
print("=== 2. 算術演算 ===")
let a = 10
let b = 3
print("10 + 3  =", a + b)
print("10 - 3  =", a - b)
print("10 * 3  =", a * b)
print("10 // 3 =", a // b)
print("10 %  3 =", a % b)
print("10 ** 3 =", a ** b)
print("2 ** 8  =", 2 ** 8)
print("10.0 / 4 =", 10.0 / 4)

print("=== 2b. 文字列連結 ===")
let greeting = "Hello" + ", " + lang + "!"
print(greeting)

# --- 3. 比較・論理演算 ---
print("=== 3. 比較・論理 ===")
let x = 42
print("x > 0 and x < 100 :", x > 0 and x < 100)
print("x == 42 or x == 0 :", x == 42 or x == 0)
print("not (x == 0)       :", not (x == 0))
print("x != 99            :", x != 99)

# --- 4. if / elif / else ---
print("=== 4. if / elif / else ===")
mut score = 85
if score >= 90:
    print("grade: A")
elif score >= 80:
    print("grade: B")
elif score >= 70:
    print("grade: C")
else:
    print("grade: D")

# ネスト if
if score > 0:
    if score >= 80:
        print("pass!")
    else:
        print("retry")

# --- 5. while + break / continue ---
print("=== 5. while ===")
mut i = 1
mut sum_odd = 0
while i <= 10:
    if i % 2 == 0:
        i += 1
        continue
    sum_odd += i
    i += 1
print("1+3+5+7+9 =", sum_odd)    # 25

mut countdown = 5
while True:
    print("countdown:", countdown)
    countdown -= 1
    if countdown == 0:
        break
print("lift off!")

# --- 6. for + range ---
print("=== 6. for + range ===")
mut total = 0
for n in range(1, 6):
    total += n
print("sum 1-5 =", total)        # 15

# step 付き range
print("even 0-8:")
for n in range(0, 10, 2):
    print(" ", n)

# リテラルリストの反復
print("fruits:")
for fruit in ["apple", "banana", "cherry"]:
    print(" -", fruit)

# --- 7. len / リスト ---
print("=== 7. len / list ===")
let nums = [10, 20, 30, 40, 50]
print("nums =", nums)
print("len  =", len(nums))

# リストの合計
mut s = 0
for v in nums:
    s += v
print("sum  =", s)               # 150

# --- 8. ネスト for + break ---
print("=== 8. ネスト for ===")
mut found = False
for row in range(3):
    for col in range(3):
        if row == 1 and col == 2:
            found = True
            break
if found:
    print("found at (1, 2)")
else:
    print("not found")

# --- 9. block スコープ ---
print("=== 9. block スコープ ===")
mut outer_val = 10

block:
    mut inner_val = 99
    outer_val = 42           # 外側の変数を変更できる
    print("inside block: outer_val =", outer_val)
    print("inside block: inner_val =", inner_val)

print("after block: outer_val =", outer_val)
# inner_val はここでは参照不可（NameError になる）

# --- 10. ビット演算 ---
print("=== 10. ビット演算 ===")
let flags = 0b1010
let mask  = 0b1100
print("0b1010 & 0b1100 =", flags & mask)    # 8  (0b1000)
print("0b1010 | 0b1100 =", flags | mask)    # 14 (0b1110)
print("0b1010 ^ 0b1100 =", flags ^ mask)    # 6  (0b0110)
print("5 << 2          =", 5 << 2)          # 20
print("20 >> 2         =", 20 >> 2)         # 5
print("~0              =", ~0)              # -1
print("0xFF            =", 0xFF)            # 255

# --- 11. pass ---
print("=== 11. pass ===")
for _ in range(3):
    pass
print("loop with pass done")

# --- 12. fn / class の構文パース確認 ---
print("=== 12. fn / class 構文パース ===")

fn add(a: int, b: int) -> int:
    return a + b

fn greet(name: str) -> str:
    return "Hello, " + name

fn factorial(mut n: int) -> int:
    mut result = 1
    while n > 1:
        result *= n
        n -= 1
    return result

class Animal:
    fn speak(self) -> str:
        pass

class Dog(Animal):
    mut name = ""
    fn __init__(mut self, name: str) -> None:
        self.name = name
    fn speak(self) -> str:
        return "Woof!"
    fn fetch(mut self, item: str) -> None:
        pass

class Stack:
    fn push(mut self, item: int) -> None:
        pass
    fn pop(mut self) -> int:
        pass
    fn is_empty(self) -> bool:
        pass

print("fn / class 構文パース: OK")

# --- 完了 ---
print("=== All checks passed! ===")
