# collection_generics_errors.tl — StaticTypeError for typed collection mismatches
# Expected: StaticTypeError at the call site

fn sum_ints(items: list[int]) -> int:
    mut total = 0
    for x in items:
        total += x
    return total

# list[str] is not compatible with list[int]
let words: list[str] = ["hello", "world"]
sum_ints(words)
