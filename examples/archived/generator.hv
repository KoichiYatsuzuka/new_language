# ジェネレータ機能のデモ

# --- 1. 基本: カウントアップジェネレータ ---
gen count_up(start: int, stop: int) -> int:
    mut i: int = start
    while i < stop:
        yield i
        i += 1

let c = count_up(1, 4)
print(c.next())   # 1
print(c.next())   # 2
print(c.next())   # 3

# --- 2. for ループ相当: リストに消費 ---
gen squares(n: int) -> int:
    mut i: int = 0
    while i < n:
        yield i * i
        i += 1

let sq = squares(5)
print(sq.next())   # 0
print(sq.next())   # 1
print(sq.next())   # 4
print(sq.next())   # 9
print(sq.next())   # 16

# --- 3. 条件分岐を含むジェネレータ ---
gen evens_only(items: list) -> int:
    for x in items:
        if x % 2 == 0:
            yield x

let ev = evens_only([1, 2, 3, 4, 5, 6])
print(ev.next())   # 2
print(ev.next())   # 4
print(ev.next())   # 6

# --- 4. パースエラー確認用コメント ---
# 以下のコードはパースエラーになります（コメントアウト済み）:
# gen bad(x: int) -> int:
#     return x          # ParseError: generator function must not contain a `return` statement
#
# gen bad2(mut x: int) -> int:
#     yield x           # ParseError: parameter `x` cannot be `mut`

print("done")
