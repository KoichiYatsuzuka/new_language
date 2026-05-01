# 基本クラス
class Animal:
    fn speak(self):
        pass

# 継承
class Dog(Animal):
    fn speak(self):
        pass

# 型アノテーション付きメソッド
class Point:
    mut x = 0
    mut y = 0

    fn __init__(mut self, x: int, y: int):
        self.x = x
        self.y = y

    fn to_str(self) -> str:
        pass

# ジェネリック型アノテーション
class Stack:
    fn push(mut self, item: int):
        pass

    fn pop(mut self) -> int:
        pass

    fn items(self) -> list[int]:
        pass

print("parse ok")
