# match__errors.tl — mixed case/is arms should produce a ParseError
# Running this file should fail at parse time with:
#   ParseError: match statement cannot mix 'case' and 'is' arms

mut x = 0
match (x):
    case 0:
        print("zero")
    is int:
        print("int")
