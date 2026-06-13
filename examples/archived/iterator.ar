# iterator.tl — イテレータの動作確認

# --- 1. Listのイテレータ（for ループ経由） ---
mut sum = 0
for x in [10, 20, 30]:
    sum += x
print(sum)   # 60

# --- 2. list.__iter__() で手動取得したジェネレータ ---
let lst = [1, 2, 3]
let it = lst.__iter__()
print(it.next())   # 1
print(it.next())   # 2
print(it.next())   # 3

# --- 3. 文字列イテレータ ---
mut chars = 0
for c in "hello":
    chars += 1
print(chars)   # 5

# --- 4. カスタムクラスに __iter__ を定義してfor文で使う ---
class NumberRange:
    mut low: int
    mut high: int

    gen __iter__(self) -> int:
        mut i = self.low
        while i <= self.high:
            yield i
            i += 1

let r = NumberRange(1, 5)
mut total = 0
for v in r:
    total += v
print(total)   # 15  (1+2+3+4+5)

# --- 5. break / continue がイテレータ越しに動作する ---
mut evens = 0
for n in [1, 2, 3, 4, 5, 6]:
    if n % 2 != 0:
        continue
    evens += n
print(evens)   # 12  (2+4+6)

mut first_three = 0
for n in [1, 2, 3, 4, 5]:
    if n > 3:
        break
    first_three += n
print(first_three)   # 6  (1+2+3)

# --- 6. range() もイテレータ経由（Listを返すので互換） ---
mut s = 0
for i in range(1, 6):
    s += i
print(s)   # 15
