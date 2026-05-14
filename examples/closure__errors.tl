# closure__errors.tl — クロージャのエラー例
# クロージャにキャプチャされた可変変数は freeze できない

fn outer() -> None:
    mut x = 0
    fn inner() -> None:
        x += 1
    freeze x  # TypeError: cannot freeze 'x' because it is captured by a closure

outer()
