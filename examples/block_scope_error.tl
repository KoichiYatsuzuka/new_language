# block_scope_error.tl — NameError: accessing a block-scoped variable outside its block
#
# Expected error (runtime NameError):
#   Variable 'inner' is declared inside a block and is not accessible after the block ends.

block:
    mut inner = 42
    print("inside block:", inner)   # 42

print(inner)   # NameError: 'inner' is not defined
