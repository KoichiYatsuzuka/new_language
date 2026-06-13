# file_io.tl — ファイル I/O 組み込み関数のデモ

# --- path 型 ---
let p = path("examples/file_io.tl")
print(p.value)              # examples/file_io.tl

# --- enum 値の確認 ---
print(FileOpenMode.read.value)        # 2
print(StartPoint.top.value)           # 0
print(ByteRecognizingMode.text.value) # 1
print(Encoding.UTF_8.value)           # 1

# --- rewrite モード: 新規作成 or 上書き ---
let f = open("examples/_tmp_demo.txt", FileOpenMode.rewrite)
f.write_line("Hello, world!")
f.write_line("Second line.")
f.write("No newline at end")
close(f)

# --- read モード: 全体を一括読み込み ---
let g = open("examples/_tmp_demo.txt", FileOpenMode.read)
let all = g.read()
print(all)
# Hello, world!
# Second line.
# No newline at end
close(g)

# --- read_line で行ごとに読み込む ---
let h = open("examples/_tmp_demo.txt", FileOpenMode.read)
let line1 = h.read_line()
let line2 = h.read_line()
let line3 = h.read_line()
close(h)
print(line1)   # Hello, world!\n
print(line2)   # Second line.\n
print(line3)   # No newline at end

# --- read_letter で1文字ずつ ---
let k = open("examples/_tmp_demo.txt", FileOpenMode.read)
let c1 = k.read_letter()
let c2 = k.read_letter()
let c3 = k.read_letter()
close(k)
print(c1)  # H
print(c2)  # e
print(c3)  # l

# --- write モード: 既存ファイルを読み書き (中間挿入) ---
# ファイル内容: "Hello, world!\nSecond line.\nNo newline at end"
# ポインタを "Hello" の後 (位置 5) に進め、" there" を挿入する
let w = open("examples/_tmp_demo.txt", FileOpenMode.write)
let _ = w.read_letter()   # H
let _ = w.read_letter()   # e
let _ = w.read_letter()   # l
let _ = w.read_letter()   # l
let _ = w.read_letter()   # o
w.write(" there")
close(w)

let v = open("examples/_tmp_demo.txt", FileOpenMode.read)
let inserted = v.read_line()
close(v)
print(inserted)   # Hello there, world!\n

# --- backward 読み込み ---
let b = open("examples/_tmp_demo.txt", FileOpenMode.read, StartPoint.end)
let last_line = b.read_line(backward = True)
close(b)
print(last_line)   # No newline at end (末尾行を逆方向に読む)

# --- make_and_write: 新規ファイル作成 ---
let nf = open("examples/_tmp_new.txt", FileOpenMode.make_and_write)
nf.write("Created fresh!")
close(nf)
let nfr = open("examples/_tmp_new.txt", FileOpenMode.read)
print(nfr.read())   # Created fresh!
close(nfr)

# --- byte モード ---
let bf = open("examples/_tmp_bytes.txt", FileOpenMode.rewrite, StartPoint.top, ByteRecognizingMode.byte)
bf.write([72, 101, 108, 108, 111])   # "Hello" in bytes
close(bf)

let br = open("examples/_tmp_bytes.txt", FileOpenMode.read, StartPoint.top, ByteRecognizingMode.byte)
let bytes = br.read()
close(br)
print(bytes)   # [72, 101, 108, 108, 111]

# クリーンアップ (make_and_write 再実行のため)
# (本来は例外処理や os モジュールを使うが、デモなので省略)
