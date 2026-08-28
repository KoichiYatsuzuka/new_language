"""py_ternary.py - `import[py]` 変換器の三項演算子対応（項目 11）の検査用モジュール。

Python の `a if cond else b` は**式**。Arrow の `if` 式は分岐本体が文の列で、
値は `block_return` で返す形なので、各腕を `block_return <値>` 1 文のブロックにして写す。
"""

CALLS = []


# --- 1. 素朴な三項式 ---
def sign(n):
    return "pos" if n > 0 else "nonpos"


# --- 2. 入れ子（括弧つき） ---
def nested(n):
    return "pos" if n > 0 else ("zero" if n == 0 else "neg")


# --- 3. 括弧なしの連鎖（右結合。elif 相当） ---
def chain(n):
    return "a" if n == 1 else "b" if n == 2 else "c" if n == 3 else "z"


# --- 4. 代入の右辺 ---
def in_assign(n):
    v = 1 if n else 2
    return v


# --- 5. 呼び出し引数の中 ---
def in_arg(n):
    return str(3 if n else 4)


# --- 6. リスト要素 / dict の値 ---
def in_list(n):
    return [1 if n else 2, 3 if n else 4]


def in_dict(c):
    return {"k": 10 if c else 20}


# --- 7. while の条件式の中 ---
def in_cond(n):
    total = 0
    while n > (0 if n < 100 else 50):
        total = total + 1
        n = n - 1
    return total


# --- 8. 腕の型が違ってもよい（Python は動的） ---
def mixed(c):
    return 1 if c else "one"


# --- 9. ★遅延評価: 選ばれた腕しか評価されない ---
def side(tag):
    CALLS.append(tag)
    return tag


def lazy(c):
    r = side("T") if c else side("F")
    return r + ":" + str(len(CALLS))
