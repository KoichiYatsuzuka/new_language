# タプルの動作確認

# 空タプル
let empty = ()
print(empty)  # ()

# 単要素タプル（末尾カンマ必須）
let single = (42,)
print(single)  # (42,)

# 複数要素タプル
let triple = (1, "hello", True)
print(triple)  # (1, 'hello', True)

# 丸括弧だけはグループ式（タプルではない）
let grouped = (10 + 20)
print(grouped)  # 30

# タプルは変数に束縛できる
let point = (3, 4)
print(point)  # (3, 4)

# 等価比較
let a = (1, 2, 3)
let b = (1, 2, 3)
let c = (1, 2, 4)
print(a == b)  # True
print(a == c)  # False
print(a != c)  # True

# 真偽値
let nonempty = (0, 0)
let nothing = ()
print(not nonempty)  # False（要素があれば truthy）
print(not nothing)   # True（空タプルは falsy）

# 多行リテラル（カンマの後で改行してもOK）
let multi = (
    100,
    200,
    300
)
print(multi)  # (100, 200, 300)

# ネストしたタプル
let nested = ((1, 2), (3, 4))
print(nested)  # ((1, 2), (3, 4))

# タプルを関数に渡す
fn show_pair(pair: tuple) -> None:
    print(pair)

show_pair((10, 20))
