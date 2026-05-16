# default_param__errors.tl — non-default parameter after default parameter
# Expected: ParseError

fn my_func(let param1: int = 0, let param2: int) -> int:
    return param1 + param2
