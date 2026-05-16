# function_closure_error.tl — TypeError: freeze on a mut variable captured by a closure
#
# Expected error (runtime TypeError):
#   A mut variable that has been captured by an inner function cannot be frozen.
#   Freezing it would invalidate the shared cell the inner function holds.

fn make_counter() -> None:
    mut count = 0
    fn inc() -> None:
        count += 1
    freeze count   # TypeError: cannot freeze 'count' because it is captured by a closure

make_counter()
