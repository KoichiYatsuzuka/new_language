from abc import ABC, abstractmethod
from typing import Self, Final

print('=== basic class ===')
class Point:
    x: int
    y: int
    def to_str(self) -> str:
        return 'Point'

    def translate(self, dx: int, dy: int) -> Self:
        return Self((self.x + dx), (self.y + dy))

    def distance_sq(self, other: Self) -> int:
        dx = (self.x - other.x)
        dy = (self.y - other.y)
        return ((dx * dx) + (dy * dy))

    def __init__(self, x: int, y: int) -> None:
        self.x = x
        self.y = y


p = Point(3, 4)
print(p.x, p.y)
q = p.translate(1, (-1))
print(q.x, q.y)
dsq = p.distance_sq(q)
print(dsq)
print('=== class with __init__ ===')
class Circle:
    radius: int
    PI: Final[int] = 3
    def __init__(self, r: int) -> None:
        self.radius = r

    def area_approx(self) -> int:
        return ((self.PI * self.radius) * self.radius)

    def scale(self, factor: int) -> None:
        self.radius = (self.radius * factor)


c = Circle(5)
print('radius:', c.radius)
print('area approx:', c.area_approx())
c.scale(2)
print('after scale:', c.radius)
print('PI constant:', c.PI)
print('=== trait ===')
class Describable(ABC):
    @abstractmethod
    def describe(self) -> str:
        ...


class Measurable(ABC):
    @abstractmethod
    def measure(self) -> int:
        ...


class Box(Describable, Measurable):
    width: int
    height: int
    def describe(self) -> str:
        return 'Box'

    def measure(self) -> int:
        return (self.width * self.height)

    def __init__(self, width: int, height: int) -> None:
        self.width = width
        self.height = height


b = Box(4, 6)
print(b.describe())
print(b.measure())
print('=== trait-qualified access ===')
class Creature(ABC):
    name: str
    hp: int
    @abstractmethod
    def speak(self) -> str:
        ...


class Wolf(Creature):
    pack_size: int
    def speak(self) -> str:
        return 'Howl!'

    def get_name(self) -> str:
        return self.name

    def get_hp(self) -> int:
        return self.hp

    def take_damage(self, dmg: int) -> None:
        self.hp = (self.hp - dmg)

    def is_alive(self) -> bool:
        return (self.hp > 0)

    def __init__(self, name: str, hp: int, pack_size: int) -> None:
        self.name = name
        self.hp = hp
        self.pack_size = pack_size


wolf = Wolf('Grey', 40, 5)
print(wolf.get_name())
print(wolf.get_hp())
print(wolf.speak())
wolf.take_damage(15)
print(wolf.get_hp())
print(wolf.is_alive())
print('=== Self type ===')
class Builder:
    value: int
    def set(self, v: int) -> Self:
        return Self(v)

    def doubled(self) -> Self:
        return Self((self.value * 2))

    def __init__(self, value: int) -> None:
        self.value = value


result = Builder(1).set(5).doubled()
print(result.value)
print('=== let fields ===')
class Token:
    kind: str
    text: str
    def __init__(self, kind: str, text: str) -> None:
        self.kind = kind
        self.text = text


t = Token('ident', 'foo')
print(t.kind, t.text)
print('=== access control ===')
class Counter:
    label: str
    count: int
    step: int
    def __init__(self, label: str) -> None:
        self.label = label
        self.count = 0
        self.step = 1

    def increment(self) -> None:
        self.count = (self.count + self.step)

    def get_count(self) -> int:
        return self.count

    def set_step(self, s: int) -> None:
        self.step = s

    def show(self) -> None:
        print(self.label)
        print(self.count)
        print(self.step)

    def __init__(self, label: str, count: int, step: int) -> None:
        self.label = label
        self.count = count
        self.step = step


ctr = Counter('hits')
print(ctr.label)
ctr.increment()
ctr.increment()
print(ctr.get_count())
ctr.set_step(5)
ctr.increment()
print(ctr.get_count())
ctr.show()
print('=== static / class_method ===')
class Registry:
    # static mut — class-level variable
    entry_count: int = 0
    def __init__(self) -> None:
        Registry.entry_count += 1

    @staticmethod
    def reset() -> None:
        Registry.entry_count = 0

    @staticmethod
    def count() -> int:
        return Registry.entry_count

    @classmethod
    def describe(cls: type[Self]) -> str:
        return cls.name


print(Registry.count())
r1 = Registry()
r2 = Registry()
print(Registry.count())
Registry.reset()
print(Registry.count())
r3 = Registry()
print(Registry.count())
print(Registry.describe())
