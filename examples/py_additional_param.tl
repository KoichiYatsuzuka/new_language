import[py] py_calculator as calc

# **kwargs 関数: キーワード引数の値が順番に出力される
calc.func(x=1, y=2, z=3)

# 引数リストにないキーワード引数 (verbose, debug) を渡してもエラーにならない
let r1 = calc.add_flexible(10, 5, verbose=True, debug=False)
print(r1)

# 通常の呼び出しも引き続き動作する
let r2 = calc.add(3, 7)
print(r2)
