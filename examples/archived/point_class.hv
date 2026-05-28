# point_class.tl — Use a C++ class (POINT) defined in test_modules/point.cpp,
# compiled to point.dll, bridged via import[cpp-dll].
#
# Build the DLL first (from a Developer Command Prompt):
#   cd examples\test_modules
#   build_point.bat
#
# Then run:
#   cargo run -- examples/point_class.tl

import[cpp-dll] test_modules.point as _pt

# POINT class is auto-generated from:
#   typedef struct { int x; int y; } POINT;
# in point.h — no manual class definition needed.

mut p1 = POINT(3, 4)
mut p2 = POINT(0, 0)

print(f"p1 = ({p1.x}, {p1.y})")   # (3, 4)
print(f"p2 = ({p2.x}, {p2.y})")   # (0, 0)

# Pass struct fields to C++ handle-based API
let h1 = _pt.point_create(p1.x, p1.y)
let h2 = _pt.point_create(p2.x, p2.y)

print(f"distance_sq = {_pt.point_distance_sq(h1, h2)}")  # 25

_pt.point_move(h1, 1, 0)
p1.x = _pt.point_get_x(h1)
p1.y = _pt.point_get_y(h1)
print(f"p1 after move(1,0) = ({p1.x}, {p1.y})")          # (4, 4)

_pt.point_set_x(h2, 4)
_pt.point_set_y(h2, 4)
p2.x = _pt.point_get_x(h2)
p2.y = _pt.point_get_y(h2)
print(f"p2 after set = ({p2.x}, {p2.y})")                # (4, 4)
print(f"distance_sq = {_pt.point_distance_sq(h1, h2)}")  # 0

_pt.point_destroy(h1)
_pt.point_destroy(h2)
print("done")
