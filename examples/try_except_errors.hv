# Internal error that is not caught — propagates as a proper traceback

let lst = [1, 2, 3]
try:
    let x = lst[99]
except ValueError:
    pass
# IndexError does not match ValueError, so it propagates uncaught
