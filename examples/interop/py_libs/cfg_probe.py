MARKER = "search_path_ok"

def probe(n):
    return n * 3 + 1

def describe():
    return "cfg_probe from py_libs"

def with_default(a, b=10):
    return a + b

def with_kwargs(a, b=1, c=2):
    return a * 100 + b * 10 + c

def varargs(*items):
    total = 0
    for it in items:
        total += it
    return total

def mutate(lst):
    lst.append(99)
    return len(lst)

class Counter:
    def __init__(self, start):
        self.value = start

    def bump(self, by):
        self.value = self.value + by
        return self.value

    def get(self):
        return self.value
