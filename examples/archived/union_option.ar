# Union / Option の動作確認サンプル

# --- Union[int, str] パラメータへの使用 ---

fn print_val(x: Union[int, str]) -> None:
    print(x)

print_val(42)        # int は Union[int, str] に渡せる
print_val("hello")   # str も Union[int, str] に渡せる

# --- 同じ Union 型の転送（Union 値 → 同一 Union 引数は OK）---

fn relay(x: Union[int, str]) -> None:
    print_val(x)   # Union[int, str] → Union[int, str] は許可

relay(100)
relay("world")

# --- Union[int, str, bool] の 3 型 ---

fn show_scalar(x: Union[int, str, bool]) -> None:
    print(x)

show_scalar(10)
show_scalar("text")
show_scalar(True)

# --- Option[int]（= Union[int, None]）---

fn maybe_print(x: Option[int]) -> None:
    print(x)

maybe_print(7)     # int は Option[int] に渡せる
maybe_print(None)  # None も Option[int] に渡せる

# --- Option 値を別の Option 引数へ転送 ---

fn forward_opt(x: Option[str]) -> None:
    print(x)

fn use_opt(x: Option[str]) -> None:
    forward_opt(x)   # Option[str] → Option[str] は許可

use_opt("hi")
use_opt(None)

# --- Union 内に Option をネストする ---

fn nested(x: Union[Option[int], str]) -> None:
    print(x)

fn call_nested(a: Option[int], b: str) -> None:
    nested(a)   # Option[int] は Union[Option[int], str] の直接メンバー
    nested(b)   # str も直接メンバー

call_nested(None, "nested ok")
