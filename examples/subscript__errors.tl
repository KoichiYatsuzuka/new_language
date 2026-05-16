## subscript__errors.tl — subscript のエラーケース
## このファイルを実行すると IndexError が発生して終了する

let xs = [1, 2, 3]
let r = xs[10]     # IndexError: list index 10 out of range
print(r)
