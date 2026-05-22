# collection_generics.tl — typed list[T], dict[K,V], set[T] annotations

# ============================================================
# 1. Typed list[T]
# ============================================================
print("=== list[T] ===")

fn sum_ints(items: list[int]) -> int:
    mut total = 0
    for x in items:
        total += x
    return total

let nums: list[int] = [1, 2, 3, 4, 5]
print("sum:", sum_ints(nums))   # 15

fn join_strs(items: list[str]) -> str:
    mut result = ""
    for s in items:
        result = result + s + " "
    return result

let words: list[str] = ["hello", "world"]
print("joined:", join_strs(words))   # hello world

# ============================================================
# 2. Typed dict[K,V]
# ============================================================
print("=== dict[K,V] ===")

fn lookup(d: dict[str,int], key: str) -> int:
    return d[key]

let scores: dict[str,int] = {"alice": 90, "bob": 85}
print("alice:", lookup(scores, "alice"))   # 90
print("bob:", lookup(scores, "bob"))       # 85

# ============================================================
# 3. Typed set[T]
# ============================================================
print("=== set[T] ===")

fn count_unique(items: set[int]) -> int:
    return len(items)

let unique: set[int] = {1, 2, 3, 2, 1}
print("unique count:", count_unique(unique))   # 3

# ============================================================
# 4. Nested generics
# ============================================================
print("=== nested ===")

fn first_row(matrix: list[list[int]]) -> list[int]:
    for row in matrix:
        return row
    return []

let matrix: list[list[int]] = [[1, 2], [3, 4]]
print("first row:", first_row(matrix))   # [1, 2]

# ============================================================
# 5. Class with typed collection fields
# ============================================================
print("=== class fields ===")

class Stats:
    mut values: list[int]

    fn max_val(self) -> int:
        mut m = 0
        for v in self.values:
            if v > m:
                m = v
        return m

    fn count(self) -> int:
        return len(self.values)

let s = Stats(values=[3, 1, 4, 1, 5, 9, 2, 6])
print("count:", s.count())           # 8
print("max:", s.max_val())           # 9

print("done")
