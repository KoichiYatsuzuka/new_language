import[rs] libm[0.2]

# Wrong arg type: str instead of float
print(libm.sin("hello"))

# Wrong arg count: pow expects 2 args
print(libm.pow(2.0))
