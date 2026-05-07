# ============================================================
# exceptions.tl — 例外処理の動作確認
# ============================================================

# --- 1. 基本的な raise / try / except ---
print("=== 1. 基本的な raise / except ===")

mut caught = 0
try:
    raise ValueError("invalid input")
except ValueError as e:
    caught = 1
    print("caught ValueError:", e.message)

print("caught:", caught)

# --- 2. 型が一致しない except は素通り ---
print("=== 2. 型が合わない except は素通り ===")

mut result = "none"
try:
    try:
        raise TypeError("wrong type")
    except ValueError:
        result = "ValueError caught"
except TypeError as e:
    result = "TypeError caught"
    print("outer caught TypeError:", e.message)

print("result:", result)

# --- 3. bare except（全例外キャッチ）---
print("=== 3. bare except ===")

mut x = 0
try:
    raise KeyError("missing key")
except:
    x = 99
    print("bare except triggered")

print("x:", x)

# --- 4. finally は常に実行される ---
print("=== 4. finally ===")

mut log = "start"
try:
    raise RuntimeError("boom")
except RuntimeError as e:
    log = "caught"
    print("caught RuntimeError:", e.message)
finally:
    log = log + "+finally"
    print("finally ran, log:", log)

print("log:", log)

# --- 5. finally は例外なしでも実行される ---
print("=== 5. finally without exception ===")

mut fin_ran = 0
try:
    fin_ran = 1
finally:
    fin_ran = 2

print("fin_ran:", fin_ran)

# --- 6. bare raise（現在の例外を再 raise）---
print("=== 6. bare raise (re-raise) ===")

mut outer_caught = 0
try:
    try:
        raise ValueError("reraise me")
    except ValueError as e:
        print("inner caught, re-raising")
        raise
except ValueError as e:
    outer_caught = 1
    print("outer caught after re-raise:", e.message)

print("outer_caught:", outer_caught)

# --- 7. 関数を通じた例外の伝播 ---
print("=== 7. 関数経由の伝播 ===")

fn risky(n: int) -> int:
    if n < 0:
        raise ValueError("negative not allowed")
    return n * 2

mut val = 0
try:
    val = risky(-1)
except ValueError as e:
    val = -99
    print("caught from function:", e.message)

print("val:", val)

# --- 8. ユーザー定義例外クラス ---
print("=== 8. ユーザー定義例外 ===")

class DomainError(Error):
    pass

mut domain_caught = 0
try:
    raise DomainError("domain rule violated")
except DomainError as e:
    domain_caught = 1
    print("caught DomainError:", e.message)

print("domain_caught:", domain_caught)

# --- 9. 例外インスタンスの各フィールド ---
print("=== 9. 例外フィールド ===")

try:
    raise ValueError("check fields")
except ValueError as e:
    print("message:", e.message)
    print("file:", e.file)
    print("line:", e.line)
    print("col:", e.col)

print("=== all done ===")
