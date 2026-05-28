# built_in.tl — built-in functions and types demo.
#
# Sections:
#   1. id()            — object identity
#   2. enumerate()     — indexed iteration
#   3. zip()           — parallel iteration
#   4. File I/O        — path, open/close, FileOpenMode, StartPoint, ByteRecognizingMode
#   5. String features — f/r/m/$-strings, %, format(), methods, regex, ord()

# ── 1. id() ──────────────────────────────────────────────────────────────────
# id() returns a pointer instance wrapping a uint value

let n = 42
let p = id(n)
print("id(42) =", str(p.value))

# Reference types: same variable → same id
class Box:
    fn __init__(mut self, let val: int) -> None:
        self.val = val

let a = Box(1)
let b = a            # let→let: same Rc, same id
let pa = id(a)
let pb = id(b)
print("id(a) == id(b):", str(pa.value == pb.value))  # True

# mut copy → new allocation → different id
mut c = a
let pc = id(c)
print("id(a) == id(c):", str(pa.value == pc.value))  # False

# Value types: equal values → equal ids (value-based identity)
let x = 0
let y = x            # let→let: no copy for value types either
let px = id(x)
let py = id(y)
print("id(x) == id(y) (int 0):", str(px.value == py.value))  # True

# uint type: id() value field is a uint
let u = uint(100)
let pu = id(u)
print("id(uint(100)) type is uint:", str(pu.value is uint))  # True

# pointer is a new_type wrapping uint
print("pointer.value of id(a):", str(pa.value))

# ── 2. enumerate() ───────────────────────────────────────────────────────────
# enumerate(iterable [, start=0]) — returns (index, value) tuples

let fruits = ["apple", "banana", "cherry"]

# Basic usage — index starts at 0
for i, name in enumerate(fruits):
    print(i, name)
# 0 apple
# 1 banana
# 2 cherry

print("---")

# start= keyword shifts the index
for i, name in enumerate(fruits, start=1):
    print(i, name)
# 1 apple
# 2 banana
# 3 cherry

print("---")

# enumerate over a tuple
let coords = (10, 20, 30)
for i, v in enumerate(coords):
    print(i, v)
# 0 10
# 1 20
# 2 30

print("---")

# ── 3. zip() ─────────────────────────────────────────────────────────────────
# zip(iter1, iter2, ...) — stops at the shortest iterable

let names = ["Alice", "Bob", "Carol"]
let scores = [95, 82, 78]

for name, score in zip(names, scores):
    print(name, score)
# Alice 95
# Bob 82
# Carol 78

print("---")

# Stops at the shortest input
let short = [1, 2]
let long = [10, 20, 30, 40]
for ea, eb in zip(short, long):
    print(ea, eb)
# 1 10
# 2 20

print("---")

# Three iterables at once
let xs = [1, 2, 3]
let ys = [4, 5, 6]
let zs = [7, 8, 9]
for ex, ey, ez in zip(xs, ys, zs):
    print(ex + ey + ez)
# 12
# 15
# 18

print("---")

# Combining enumerate and zip
let za = [10, 20, 30]
let zb = [40, 50, 60]
for i, pair in enumerate(zip(za, zb)):
    let zx, let zy = pair
    print(i, zx, zy)
# 0 10 40
# 1 20 50
# 2 30 60

# ── 4. File I/O ───────────────────────────────────────────────────────────────
# path type
let fp = path("examples/built_in.tl")
print(fp.value)              # examples/built_in.tl

# enum values
print(FileOpenMode.read.value)        # 2
print(StartPoint.top.value)           # 0
print(ByteRecognizingMode.text.value) # 1
print(Encoding.UTF_8.value)           # 1

# rewrite mode: create or overwrite
let f = open("examples/_tmp_demo.txt", FileOpenMode.rewrite)
f.write_line("Hello, world!")
f.write_line("Second line.")
f.write("No newline at end")
close(f)

# read mode: read all at once
let g = open("examples/_tmp_demo.txt", FileOpenMode.read)
let all = g.read()
print(all)
# Hello, world!
# Second line.
# No newline at end
close(g)

# read_line — line by line
let h = open("examples/_tmp_demo.txt", FileOpenMode.read)
let line1 = h.read_line()
let line2 = h.read_line()
let line3 = h.read_line()
close(h)
print(line1)   # Hello, world!\n
print(line2)   # Second line.\n
print(line3)   # No newline at end

# read_letter — one character at a time
let k = open("examples/_tmp_demo.txt", FileOpenMode.read)
let c1 = k.read_letter()
let c2 = k.read_letter()
let c3 = k.read_letter()
close(k)
print(c1)  # H
print(c2)  # e
print(c3)  # l

# write mode: read-write (mid-file insertion)
let w = open("examples/_tmp_demo.txt", FileOpenMode.write)
let _ = w.read_letter()   # H
let _ = w.read_letter()   # e
let _ = w.read_letter()   # l
let _ = w.read_letter()   # l
let _ = w.read_letter()   # o
w.write(" there")
close(w)

let vf = open("examples/_tmp_demo.txt", FileOpenMode.read)
let inserted = vf.read_line()
close(vf)
print(inserted)   # Hello there, world!\n

# backward read — start from end of file
let bk = open("examples/_tmp_demo.txt", FileOpenMode.read, StartPoint.end)
let last_line = bk.read_line(backward = True)
close(bk)
print(last_line)   # No newline at end

# make_and_write: create a new file
let nf = open("examples/_tmp_new.txt", FileOpenMode.make_and_write)
nf.write("Created fresh!")
close(nf)
let nfr = open("examples/_tmp_new.txt", FileOpenMode.read)
print(nfr.read())   # Created fresh!
close(nfr)

# byte mode
let bf = open("examples/_tmp_bytes.txt", FileOpenMode.rewrite, StartPoint.top, ByteRecognizingMode.byte)
bf.write([72, 101, 108, 108, 111])   # "Hello" in bytes
close(bf)

let br = open("examples/_tmp_bytes.txt", FileOpenMode.read, StartPoint.top, ByteRecognizingMode.byte)
let bytes = br.read()
close(br)
print(bytes)   # [72, 101, 108, 108, 111]

# ── 5. String features ────────────────────────────────────────────────────────

# f-string (interpolation)
let name = "Alice"
let age = 30
print(f"Hello, {name}! You are {age} years old.")

let pi_val = 3.14159
print(f"pi ≈ {pi_val}")

let items = [1, 2, 3]
print(f"list: {items}")

# r-string (raw — backslashes kept as-is)
let raw = r"\n is not a newline in raw strings"
print(raw)

let win_path = r"C:\Users\Alice\Documents"
print(win_path)

# m"..." math string (LaTeX-style Unicode conversion)
print(m"Pythagorean theorem: a^2 + b^2 = c^2")
print(m"Euler's identity: e^{i\pi} + 1 = 0")
print(m"Subscript example: x_0, x_1, x_2")
print(m"Greek letters: \alpha, \beta, \gamma, \delta, \pi, \omega")
print(m"Operators: a \times b, a \div b, a \pm b, a \neq b, a \leq b")
print(m"Calculus: \int f(x) dx, \sum_{i=0}^{n} x_i, \partial f / \partial x")

# $...$ math string (compact notation)
print($x^2 + y^2 = z^2$)
print($E = mc^2$)
print($F = G \cdot m_1 m_2 / r^2$)

# % format operator
let pi = 3.14159265
print("pi = %.4f" % pi)
print("int: %d, hex: %x, oct: %o" % (255, 255, 255))
print("name: %s, age: %d" % ("Bob", 25))
print("scientific: %e" % 0.000123)
print("padded: '%10s'" % "hi")
print("left-aligned: '%-10s|'" % "hi")
print("zero-padded: '%05d'" % 42)

# str.format()
print("Hello, {}! You are {} years old.".format("Carol", 22))
print("{0} + {1} = {2}".format(1, 2, 3))
print("pi = {:.3f}".format(3.14159))
print("{name} scored {score:05d}".format(name="Dave", score=99))
print("{:>10}".format("right"))
print("{:<10}|".format("left"))
print("{:^10}".format("center"))

# String methods
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

print("123".isdigit())
print("abc".isalpha())
print("abc123".isalnum())
print("   ".isspace())
print("HELLO".isupper())
print("hello".islower())

print("42".zfill(6))
print("hi".ljust(10, "*"))
print("hi".rjust(10, "*"))
print("hi".center(10, "-"))

let multiline = "line1\nline2\nline3"
print(multiline.splitlines())

print("a\tb\tc".expandtabs(4))

let str_bytes = "Hi".encode()
print(str_bytes)

# * repetition operator
print("ha" * 3)
print(3 * "na" + " Batman!")

# Regex methods
let text = "Hello, my phone is 090-1234-5678 and email is alice@example.com"

let phone = text.search(r"\d{3}-\d{4}-\d{4}")
print(phone)

let digits = "abc 123 def 456 ghi 789".findall(r"\d+")
print(digits)

let cleaned = "foo123bar456".sub(r"\d+", "NUM")
print(cleaned)

let re_words = "one two  three   four".regex_split(r"\s+")
print(re_words)

let matched = "hello world".match(r"hello")
print(matched)

let no_match = "hello world".match(r"world")
print(no_match)

let ci = "Hello World".findall(r"hello", "i")
print(ci)

# ord() — character to code point
print("A".ord())
print("α".ord())
