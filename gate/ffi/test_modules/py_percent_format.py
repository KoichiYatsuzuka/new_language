"""py_percent_format.py - `%` の書式の `%s` / `%r`（known_bugs.md の #4）。"""


class T:
    def __repr__(self):
        return "T()"

    def __str__(self):
        return "t"


class R:
    def __repr__(self):
        return "R()"


def run():
    cases = [
        ("%s|%r", (T(), T())),
        ("%s|%r", (R(), R())),
        ("%s|%r", ("ab", "ab")),
        ("%s|%r", (5, 5)),
        ("%s|%r", (1.5, 1.5)),
        ("%s|%r", (None, None)),
        ("%s|%r", (True, True)),
        ("%s|%r", ([1, "a"], [1, "a"])),
        ("%s|%r", ({"k": T()}, {"k": T()})),
        ("%s|%r", ((1, T()), (1, T()))),
        ("%r", ("it's",)),
        ("%5s|%-5s|%.1s", (T(), R(), T())),
        ("%s", T()),
        ("%s", [T()]),
    ]
    for fmt, args in cases:
        try:
            print(fmt % args)
        except Exception as e:
            print(type(e).__name__ + ":", e)
