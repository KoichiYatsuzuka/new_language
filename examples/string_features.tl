# string_features.tl — 文字列機能デモ
#
# 実行方法:
#   cargo run -- examples/string_features.tl

# ─────────────────────────────────────────────────
# 1. f-string (補間文字列)
# ─────────────────────────────────────────────────
let name = "Alice"
let age = 30
let greeting = f"Hello, {name}! You are {age} years old."
print(greeting)

let x = 3.14159
print(f"pi ≈ {x}")

let items = [1, 2, 3]
print(f"list: {items}")

# ─────────────────────────────────────────────────
# 2. r-string (生文字列 — バックスラッシュをそのまま保持)
# ─────────────────────────────────────────────────
let raw = r"\n is not a newline in raw strings"
print(raw)

let path = r"C:\Users\Alice\Documents"
print(path)

# ─────────────────────────────────────────────────
# 3. 数学文字列 m"..." (LaTeX 風 Unicode 変換)
# ─────────────────────────────────────────────────
print(m"Pythagorean theorem: a^2 + b^2 = c^2")
print(m"Euler's identity: e^{i\pi} + 1 = 0")
print(m"Subscript example: x_0, x_1, x_2")
print(m"Greek letters: \alpha, \beta, \gamma, \delta, \pi, \omega")
print(m"Operators: a \times b, a \div b, a \pm b, a \neq b, a \leq b")
print(m"Calculus: \int f(x) dx, \sum_{i=0}^{n} x_i, \partial f / \partial x")

# ─────────────────────────────────────────────────
# 4. $...$ 数学文字列 (コンパクト記法)
# ─────────────────────────────────────────────────
print($x^2 + y^2 = z^2$)
print($E = mc^2$)
print($F = G \cdot m_1 m_2 / r^2$)

# ─────────────────────────────────────────────────
# 5. % 書式演算子
# ─────────────────────────────────────────────────
let pi = 3.14159265
print("pi = %.4f" % pi)
print("int: %d, hex: %x, oct: %o" % (255, 255, 255))
print("name: %s, age: %d" % ("Bob", 25))
print("scientific: %e" % 0.000123)
print("padded: '%10s'" % "hi")
print("left-aligned: '%-10s|'" % "hi")
print("zero-padded: '%05d'" % 42)

# ─────────────────────────────────────────────────
# 6. str.format() メソッド
# ─────────────────────────────────────────────────
print("Hello, {}! You are {} years old.".format("Carol", 22))
print("{0} + {1} = {2}".format(1, 2, 3))
print("pi = {:.3f}".format(3.14159))
print("{name} scored {score:05d}".format(name="Dave", score=99))
print("{:>10}".format("right"))
print("{:<10}|".format("left"))
print("{:^10}".format("center"))

# ─────────────────────────────────────────────────
# 7. 文字列メソッド
# ─────────────────────────────────────────────────
let s = "  Hello, World!  "
print(s.strip())
print(s.lstrip())
print(s.rstrip())
print("hello world".upper())
print("HELLO WORLD".lower())
print("hello world".title())
print("Hello World".swapcase())
print("hello".capitalize())

let csv = "one,two,three,four"
let parts = csv.split(",")
print(parts)
print(",".join(["a", "b", "c"]))
print("hello world python".split(" ", 1))
print("a::b::c".rsplit("::", 1))

print("Hello, World!".find("World"))
print("Hello, World!".rfind("l"))
print("banana".count("an"))
print("Hello".startswith("He"))
print("Hello".endswith("lo"))
print("hello world".replace("world", "Python"))
print("hello world".replace("l", "L", 1))

print("Hello, World!".removeprefix("Hello, "))
print("Hello, World!".removesuffix(", World!"))

let part = "one:two:three".partition(":")
print(part)

# 文字型判定
print("123".isdigit())
print("abc".isalpha())
print("abc123".isalnum())
print("   ".isspace())
print("HELLO".isupper())
print("hello".islower())

# 幅揃え
print("42".zfill(6))
print("hi".ljust(10, "*"))
print("hi".rjust(10, "*"))
print("hi".center(10, "-"))

# splitlines
let multiline = "line1\nline2\nline3"
print(multiline.splitlines())

# expandtabs
print("a\tb\tc".expandtabs(4))

# encode (UTF-8 バイト列)
let bytes = "Hi".encode()
print(bytes)

# ─────────────────────────────────────────────────
# 8. * 繰り返し演算子
# ─────────────────────────────────────────────────
print("ha" * 3)
print(3 * "na" + " Batman!")

# ─────────────────────────────────────────────────
# 9. 正規表現メソッド
# ─────────────────────────────────────────────────
let text = "Hello, my phone is 090-1234-5678 and email is alice@example.com"

let phone = text.search(r"\d{3}-\d{4}-\d{4}")
print(phone)

let digits = "abc 123 def 456 ghi 789".findall(r"\d+")
print(digits)

let cleaned = "foo123bar456".sub(r"\d+", "NUM")
print(cleaned)

let words = "one two  three   four".regex_split(r"\s+")
print(words)

let matched = "hello world".match(r"hello")
print(matched)

let no_match = "hello world".match(r"world")
print(no_match)

# Case-insensitive flag
let ci = "Hello World".findall(r"hello", "i")
print(ci)

# ─────────────────────────────────────────────────
# 10. ord() メソッド (文字 → コードポイント)
# ─────────────────────────────────────────────────
print("A".ord())
print("α".ord())
