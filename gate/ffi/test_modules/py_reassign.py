"""py_reassign.py — 同じ名前へ何度も代入する関数（py_reassign.ar が CPython で動かす）。"""

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


# --- 9. ループの最後の値を取る（外側に既定値あり）---
#     ⚠ ループ変数そのものをループ後に読むのではなく、**別の変数へ退避**させる。
#       CPython でも Arrow でも同じ結果になる、こちらが推奨形。
def latest_with_default(xs):
    latest = -1
    for x in xs:
        latest = x
    return latest


# --- 10. ループの最後の値を取る（既定値なし）---
def latest_of(xs):
    latest = None
    for x in xs:
        latest = x
    return latest


# --- 11. `if __name__ == "__main__":` の中（import したときは走らない） ---
if __name__ == "__main__":
    scratch = basic()
    print(scratch)
