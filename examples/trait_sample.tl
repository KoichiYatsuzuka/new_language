# trait_sample.tl
# trait を使ったサンプルプログラム
#
# Animal trait を定義し、Dog / Cat に継承させる。
# - trait フィールドは self::Animal.name などでアクセスする
# - 仮想関数 speak / attack_power は各クラスでオーバーライドする

# ── 1. Trait 定義 ──────────────────────────────────────────────────────
trait Animal:
    mut name: str
    mut hp: int

    # 仮想関数：継承クラスで必ずオーバーライドが必要
    fn speak(self) -> str:
        ...

    fn attack_power(self) -> int:
        ...

# ── 2. Dog クラス ─────────────────────────────────────────────────────
class Dog(Animal):
    mut breed: str   # Dog 固有のフィールド

    fn speak(self) -> str:
        return "Woof!"

    fn attack_power(self) -> int:
        return 10

    fn get_name(self) -> str:
        return self::Animal.name

    fn get_hp(self) -> int:
        return self::Animal.hp

    fn get_breed(self) -> str:
        return self.breed

    fn take_damage(mut self, dmg: int) -> None:
        self::Animal.hp = self::Animal.hp - dmg

    fn is_alive(self) -> bool:
        return self::Animal.hp > 0

# ── 3. Cat クラス ─────────────────────────────────────────────────────
class Cat(Animal):
    mut indoor: bool   # Cat 固有のフィールド

    fn speak(self) -> str:
        return "Meow!"

    fn attack_power(self) -> int:
        return 5

    fn get_name(self) -> str:
        return self::Animal.name

    fn get_hp(self) -> int:
        return self::Animal.hp

    fn is_indoor(self) -> bool:
        return self.indoor

    fn take_damage(mut self, dmg: int) -> None:
        self::Animal.hp = self::Animal.hp - dmg

    fn is_alive(self) -> bool:
        return self::Animal.hp > 0

# ── 4. 実行 ───────────────────────────────────────────────────────────
print("=== Animal trait サンプル ===")
print("")

# combined __init__: (name: str, hp: int, breed: str)  ← trait フィールド先、クラスフィールド後
let dog = Dog("Rex", 50, "Labrador")

# combined __init__: (name: str, hp: int, indoor: bool)
let cat = Cat("Whiskers", 30, True)

print("--- 基本情報 ---")
print("Dog name :")
print(dog.get_name())
print("Dog breed:")
print(dog.get_breed())
print("Dog HP   :")
print(dog.get_hp())
print("Dog speak:")
print(dog.speak())
print("")
print("Cat name   :")
print(cat.get_name())
print("Cat indoor :")
print(cat.is_indoor())
print("Cat HP     :")
print(cat.get_hp())
print("Cat speak  :")
print(cat.speak())
print("")

print("--- バトルシミュレーション ---")

print(">>> Dog attacks Cat!")
cat.take_damage(dog.attack_power())
print("Cat HP:")
print(cat.get_hp())

print(">>> Cat attacks Dog!")
dog.take_damage(cat.attack_power())
print("Dog HP:")
print(dog.get_hp())

print(">>> Dog attacks Cat again!")
cat.take_damage(dog.attack_power())
print("Cat HP:")
print(cat.get_hp())

print("")
print("--- 生存確認 ---")
print("Dog alive:")
print(dog.is_alive())
print("Cat alive:")
print(cat.is_alive())

print("")
print("--- 致命的ダメージ ---")
cat.take_damage(100)
print("Cat HP after 100 damage:")
print(cat.get_hp())
print("Cat alive:")
print(cat.is_alive())

print("")
print("=== 完了 ===")
