# decorator.tl — デコレータ機能のデモ

# --- 1. 関数をデコレータとして使う ---

# 最も基本的な形: function を受け取って function を返す
fn log(let f: function) -> function:
    fn wrapper() -> None:
        print("calling function")
        f()
    return wrapper

@log
fn hello() -> None:
    print("hello!")

hello()
# calling function
# hello!

# --- 2. スタックされたデコレータ（下から順に適用）---

fn double_log(let f: function) -> function:
    fn wrapper() -> None:
        print("double log")
        f()
    return wrapper

@double_log
@log
fn hi() -> None:
    print("hi!")

hi()
# double log
# calling function
# hi!

# --- 3. クラスをデコレータとして使う（関数に適用）---
# クラスデコレータは __init__(mut self, let f: function) と
# __call__(self)->function を持つ必要がある。
# self.field = value は __init__ で mut self が必要。
# フィールドに保存した関数を呼ぶ場合は、一度ローカル変数に取り出してから呼ぶ。

class Retry:
    fn __init__(mut self, let f: function) -> None:
        self.original = f

    fn __call__(self) -> function:
        let original_fn = self.original  # フィールドをローカルに取り出す
        fn wrapper() -> None:
            print("retrying...")
            original_fn()
        return wrapper

@Retry
fn flaky() -> None:
    print("flaky ran")

# flaky は Retry インスタンスになっている。
# flaky() を呼ぶと __call__ が実行されラッパー関数が返る。
let wrapped_flaky = flaky()
wrapped_flaky()
# retrying...
# flaky ran

# --- 4. クラスをデコレータとして使う（クラスに適用）---
# __init__(mut self, let cls: type) と __call__(self)->type を持つ。

class Singleton:
    fn __init__(mut self, let cls: type) -> None:
        self.cls = cls

    fn __call__(self) -> type:
        return self.cls

@Singleton
class Config:
    mut value: int

# Config は Singleton インスタンスになっている。
# Config() を呼ぶと __call__ が実行され元のクラスが返る。
let original_cls = Config()
let instance = original_cls(42)
print(instance.value)
# 42
