# class_trait.tl — class definitions, traits, and implementation

# --- Basic class with auto-generated constructor ---
# Fields without defaults become positional constructor parameters in order.
print("=== basic class ===")

class Point:
    mut x: int
    mut y: int

    fn to_str(self) -> str:
        return "Point"

    fn translate(self, dx: int, dy: int) -> Self:
        return Self(self.x + dx, self.y + dy)

    fn distance_sq(self, other: Self) -> int:
        let dx = self.x - other.x
        let dy = self.y - other.y
        return dx * dx + dy * dy

let p = Point(3, 4)
print(p.x, p.y)

let q = p.translate(1, -1)
print(q.x, q.y)  # 4 3

let dsq = p.distance_sq(q)
print(dsq)  # (3-4)^2 + (4-3)^2 = 1 + 1 = 2

# --- Class with explicit __init__ ---
print("=== class with __init__ ===")

class Circle:
    mut radius: int
    const PI: int = 3

    fn __init__(mut self, r: int) -> None:
        self.radius = r

    fn area_approx(self) -> int:
        return self.PI * self.radius * self.radius

    fn scale(mut self, factor: int) -> None:
        self.radius = self.radius * factor

mut c = Circle(5)
print("radius:", c.radius)         # 5
print("area approx:", c.area_approx())  # 75
c.scale(2)
print("after scale:", c.radius)   # 10

# --- const field (class-level, shared) ---
print("PI constant:", c.PI)       # 3

# --- Trait definition ---
print("=== trait ===")

trait Describable:
    fn describe(self) -> str:
        ...

trait Measurable:
    fn measure(self) -> int:
        ...

# --- Class implementing a trait ---
class Box(Describable, Measurable):
    let width: int
    let height: int

    fn describe(self) -> str:
        return "Box"

    fn measure(self) -> int:
        return self.width * self.height

let b = Box(4, 6)
print(b.describe())  # Box
print(b.measure())   # 24

# --- Trait-qualified access: explicit read/write of trait fields ---
print("=== trait-qualified access ===")

trait Creature:
    mut name: str
    mut hp: int
    fn speak(self) -> str:
        ...

class Wolf(Creature):
    mut pack_size: int

    fn speak(self) -> str:
        return "Howl!"

    fn get_name(self) -> str:
        return self::Creature.name

    fn get_hp(self) -> int:
        return self::Creature.hp

    fn take_damage(mut self, dmg: int) -> None:
        self::Creature.hp = self::Creature.hp - dmg

    fn is_alive(self) -> bool:
        return self::Creature.hp > 0

# Trait fields come first in the auto-constructor, then class fields:
# Wolf(name: str, hp: int, pack_size: int)
mut wolf = Wolf("Grey", 40, 5)
print(wolf.get_name())   # Grey
print(wolf.get_hp())     # 40
print(wolf.speak())      # Howl!
wolf.take_damage(15)
print(wolf.get_hp())     # 25
print(wolf.is_alive())   # True

# --- Self type for method chaining ---
print("=== Self type ===")

class Builder:
    mut value: int

    fn set(self, v: int) -> Self:
        return Self(v)

    fn doubled(self) -> Self:
        return Self(self.value * 2)

let result = Builder(1).set(5).doubled()
print(result.value)  # 10

# --- Immutable let fields (set at construction, never changed) ---
print("=== let fields ===")

class Token:
    let kind: str
    let text: str

let t = Token("ident", "foo")
print(t.kind, t.text)  # ident foo
# t.kind = "op"  # TypeError: cannot assign to immutable field 'kind'
