# if / elif / else
mut score = 75
if score >= 90:
    print("A")
elif score >= 70:
    print("B")
else:
    print("C")

# while with break
mut n = 1
while n <= 10:
    if n == 5:
        break
    n += 1
print("n =", n)

# for with range
mut total = 0
for i in range(1, 6):
    total += i
print("1+2+3+4+5 =", total)

# for with list literal
for x in [10, 20, 30]:
    print(x)

# for with continue
for i in range(10):
    if i % 2 == 0:
        continue
    print(i)

# nested block scope
block:
    let msg = "hello from block"
    print(msg)

# len builtin
let items = [1, 2, 3, 4, 5]
print("len =", len(items))
