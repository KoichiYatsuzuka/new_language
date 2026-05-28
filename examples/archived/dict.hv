# dict の動作確認

# --- 辞書リテラル (dict[Any, Any]) ---
let empty = {}
let scores = {"alice": 95, "bob": 82, "carol": 77}

print(scores["alice"])   # 95
print(scores["bob"])     # 82

# --- 新しいキーの追加 ---
mut d = {"x": 10}
d["y"] = 20
d["z"] = 30
print(d["y"])   # 20
print(d["z"])   # 30

# --- 既存キーの上書き ---
d["x"] = 99
print(d["x"])   # 99

# --- key() / item() メソッド ---
let ks = d.key()
let vs = d.item()
print(len(ks))   # 3
print(len(vs))   # 3

# --- dict[K, V]() 型付きコンストラクタ ---
mut typed = dict[str, int]()
typed["one"] = 1
typed["two"] = 2
print(typed["one"])   # 1

# --- dict[K, V](literal) リテラルから型付き辞書を作成 ---
let from_lit = dict[int, str]({1: "first", 2: "second"})
print(from_lit[1])   # first
print(from_lit[2])   # second

# --- int -> float アップキャスト (型が緩い場合は許容) ---
mut floats = dict[str, float]()
floats["pi"] = 3   # int は float として受け付ける
print(floats["pi"])   # 3

# --- is_truthy ---
if empty:
    print("empty is truthy")
else:
    print("empty is falsy")   # こちらが実行される

if scores:
    print("scores is truthy")   # こちらが実行される

# --- 複数行リテラル (コンマの後の改行は無視) ---
let multi = {
    "a": 1,
    "b": 2,
    "c": 3
}
print(multi["c"])   # 3

# --- for ループで key() を走査 ---
let table = {"red": 1, "green": 2, "blue": 3}
mut total = 0
for k in table.key():
    total += table[k]
print(total)   # 6
