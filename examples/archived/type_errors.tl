# StaticTypeError を複数起こすサンプル

# --- 1. 不変変数への代入 ---
let x = 10
x = 20              # StaticTypeError: immutable 'x'

const LIMIT = 100
LIMIT = 200         # StaticTypeError: immutable 'LIMIT'

# --- 2. 複合代入も不変変数はNG ---
let count = 0
count += 1          # StaticTypeError: immutable 'count'

# --- 3. 型の不一致な順序比較 ---
let name = "Alice"
let age  = 30
print(name < age)   # StaticTypeError: str < int

let flag = True
print(flag > 0)     # StaticTypeError: bool > int

# --- 4. ifブロック内でも検査される ---
mut ok_var = 1
let frozen = 99
if ok_var > 0:
    frozen = 42     # StaticTypeError: immutable 'frozen'
