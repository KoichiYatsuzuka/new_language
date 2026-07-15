import copy

print('=== user raise ===')
try:
    raise ValueError('bad input')
except ValueError as e:
    print('caught:', e.message)
print('=== TypeError ===')
result = ''
try:
    x = ('hello' + 42)
except TypeError as e:
    result = ('caught TypeError: ' + e.message)
print(copy.deepcopy(result))
print('=== IndexError ===')
lst = [10, 20, 30]
try:
    y = lst[99]
except IndexError as e:
    print('caught IndexError:', e.message)
print('=== KeyError ===')
d = {'a': 1, 'b': 2}
try:
    z = d['missing']
except KeyError as e:
    print('caught KeyError:', e.message)
print('=== ZeroDivisionError ===')
try:
    q = (10 / 0)
except ZeroDivisionError as e:
    print('caught ZeroDivisionError:', e.message)
print('=== NameError ===')
try:
    v = no_such_variable
except NameError as e:
    print('caught NameError:', e.message)
print('=== through function ===')
def risky_div(a: int, b: int) -> int:
    return (a / b)

try:
    risky_div(5, 0)
except ZeroDivisionError as e:
    print('caught from function:', e.message)
print('=== bare except ===')
try:
    w = lst[100]
except:
    print('bare except fired')
print('=== finally ===')
fin = 'not run'
try:
    _ = (1 / 0)
except ZeroDivisionError:
    print('handler ran')
finally:
    fin = 'ran'
print('finally:', copy.deepcopy(fin))
print('=== propagation (no handler) — next line raises ===')
try:
    try:
        _ = lst[50]
    except ValueError:
        print('wrong handler — should not print')
except IndexError as e:
    print('outer caught IndexError:', e.message)
print('done')
