"""py_reassign.py - `import[py]` 変換器の変数再代入（項目 2 / INF-A）の検査用モジュール。

Python は「スコープ内で一度でも代入された名前は 1 つの変数」。
Arrow は宣言 `mut` と再代入が別ノードで、二重宣言は `already declared` になる。
変換器は**スコープ単位で全代入名を巻き上げ**、以降をすべて再代入に落とす。
"""

# --- 7. モジュール直下の再代入 ---
COUNT = 0
COUNT = COUNT + 5
COUNT = COUNT * 2


# --- 1. 素朴な再代入 ---
def basic():
    x = 1
    x = 2
    x = x + 10
    return x


# --- 2. while のカウンタ（毎回 while 本体に降りる） ---
def loop(n):
    total = 0
    while n > 0:
        total = total + n
        n = n - 1
    return total


# --- 3. for + if のネスト（旧実装はここで巻き上げ集合を失っていた） ---
def in_for(xs):
    acc = 0
    for v in xs:
        acc = acc + v
        if v > 2:
            acc = acc + 100
    return acc


# --- 4. try / except / finally の各節 ---
def in_try():
    r = 0
    try:
        r = 1
        raise ValueError("x")
    except ValueError:
        r = 2
    finally:
        r = r + 10
    return r


# --- 5. if / elif / else（旧実装が唯一カバーしていた形。退行していないこと） ---
def branches(k):
    m = 0
    if k > 0:
        m = 1
        if k > 5:
            m = 2
    else:
        m = -1
    return m


# --- 6. パラメータへの再代入（パラメータは巻き上げず、代入だけ再代入にする） ---
def param_reassign(a):
    a = a * 2
    a = a + 1
    return a


# --- 8. 入れ子関数は別スコープ（内側の x は外側を潰さない） ---
def shadow():
    x = 1

    def inner():
        x = 99
        return x

    y = inner()
    return x * 100 + y


# --- 9. ★ for のループ変数と同名の代入があると結果が変わる（下の ⑨ 参照） ---
def loop_var_collision(xs):
    i = -1
    for i in xs:
        pass
    return i


# --- 10. ループ変数だけ（代入なし）なら Python と同じくループ後も残る ---
def loop_var_only(xs):
    for i in xs:
        pass
    return i
