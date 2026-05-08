# iterator__errors.tl — EndOfIteration エラーの発生確認
# イテレータが終端を超えて next() を呼ぶと EndOfIteration エラーを送出する

let lst = [42]
let it = lst.__iter__()
let _ = it.next()   # 正常: 42 を返す
it.next()           # EndOfIteration: generator is exhausted
