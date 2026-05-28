# decorator__errors.tl — デコレータに不正なシグネチャを持つ場合の StaticTypeError

# 関数デコレータが function を返さない（int を返す）
fn bad_return(let f: function) -> int:
    return 0

@bad_return
fn my_func(let a: int) -> int:
    return a
