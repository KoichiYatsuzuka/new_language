# closure.tl — クロージャの動作例

# --- 不変変数のキャプチャ ---
# 外側スコープの let 変数は定義時点の値がディープコピーされる

fn make_greeter(let name: str) -> function[]->str:
    fn greet() -> str:
        return "Hello, " + name
    return greet

let greet_alice = make_greeter("Alice")
let greet_bob   = make_greeter("Bob")
print(greet_alice())   # Hello, Alice
print(greet_bob())     # Hello, Bob

# --- 可変変数のキャプチャ（呼び出しごとに独立した環境） ---
# mut 変数は共有セルとしてキャプチャされる
# make_counter() を呼ぶたびに新しいカウンター環境が生成される

fn make_counter() -> function[]->int:
    mut count = 0
    fn inc() -> int:
        count += 1
        return count
    return inc

let counter_a = make_counter()
let counter_b = make_counter()

print(counter_a())  # 1
print(counter_a())  # 2
print(counter_b())  # 1  (counter_b は counter_a と独立した環境を持つ)
print(counter_a())  # 3

# --- 内側関数が外側関数の実行中に変数を変更する ---

fn accumulate() -> int:
    mut total = 0
    fn add(let n: int) -> None:
        total += n
    add(10)
    add(20)
    add(30)
    return total

let result = accumulate()
print(result)  # 60

# --- static 変数：全呼び出しで同じセルを共有 ---
# static mut は宣言位置に紐付いた単一のセルを持つ
# make_counter() を何度呼んでも同じカウンター変数が使われる

fn make_static_counter() -> function[]->int:
    static mut count = 0
    fn inc() -> int:
        count += 1
        return count
    return inc

let sc_a = make_static_counter()
let sc_b = make_static_counter()  # 同じ count セルを共有する

print(sc_a())  # 1
print(sc_b())  # 2  (sc_a と sc_b が同じ count を参照)
print(sc_a())  # 3

# --- ネストしたクロージャ ---

fn adder(let x: int) -> function[let int]->int:
    fn add(let y: int) -> int:
        return x + y
    return add

let add5 = adder(5)
let add10 = adder(10)
print(add5(3))   # 8
print(add10(3))  # 13
