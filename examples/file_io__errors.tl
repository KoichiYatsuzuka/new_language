# file_io__errors.tl — ファイル I/O の期待されるエラー動作
#
# このファイルを実行すると RuntimeError または IOError で終了する。
# 各エラーを個別にテストする場合はコメントアウトして確認すること。

# --- EOF エラー: ファイル末尾でさらに読もうとする ---
# ファイルを末尾 (StartPoint.end) で開いて read を呼ぶと EOFError が発生する
let f = open("examples/_tmp_demo.txt", FileOpenMode.read, StartPoint.end)
let r = f.read()   # EOFError: EOF
close(f)
