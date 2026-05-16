# variable_error.tl — StaticTypeError: assigning to immutable variables
#
# Expected errors (all StaticTypeError, reported together before execution):
#   - reassign to let variable
#   - reassign to const variable
#   - compound assignment on let variable

let x = 10
x = 20          # StaticTypeError: immutable 'x'

const LIMIT = 100
LIMIT = 200     # StaticTypeError: immutable 'LIMIT'

let count = 0
count += 1      # StaticTypeError: immutable 'count'
