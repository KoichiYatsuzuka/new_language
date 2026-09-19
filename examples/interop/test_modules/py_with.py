"""py_with.py — `with` 文（`__exit__` を持たない場合のみ・項目 25）。

`with EXPR as x: body` を `block: mut x = EXPR; body` へ脱糖する。Arrow は
ブロック退出でローカルを破棄し、ネイティブなリソース型は `Drop` で実クリーンアップ
される（`FileData::drop` が `close()` を呼ぶ）。

⚠⚠ **ブロック退出で解放されるのは最上位だけ**。関数の中では**関数を抜けるまで**
解放されない（純 Arrow で再現する既存の制限）。⇒ 書いた直後に**同じ関数の中で**
読み直す形は空に見える。この例題は関数を抜けてから読む形にしてある。

⚠ `open` は Arrow のシグネチャ（`FileOpenMode` 列挙）なので、モードは呼び出し側から
  渡している。Python の文字列モード（`"w"`）は Arrow の `open` では通らない。
"""


def write_hello(p, mode_w):
    with open(p, mode_w) as f:
        f.write("hello")
    return "written"


def write_two(p, q, mode_w):
    # 複数アイテム（宣言順に評価される）
    with open(p, mode_w) as a, open(q, mode_w) as b:
        a.write("A")
        b.write("B")
    return "written2"


def no_binding(p, mode_w):
    # `as` なし。一時変数へ束縛してブロック退出で破棄させる。
    with open(p, mode_w):
        pass
    return "no-as"
