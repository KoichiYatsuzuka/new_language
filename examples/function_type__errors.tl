# function type — intentional StaticTypeError examples

fn make_typed() -> function[let int]->int:
    fn inner(let x: int) -> int:
        return x
    return inner

let y = make_typed()
let w = y("str")   # StaticTypeError: argument 0 of 'y' expects 'int' but got 'str'
