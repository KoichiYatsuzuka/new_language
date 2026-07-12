// vec_math.c — implementation for the cpp_struct_ptr examples.
// Built into vec_math_x64.lib by build_vec_math.ps1 (the `_x64.lib` suffix is
// required: the cpp bridge only links header-adjacent libs matching
// lib_patterns, default ["_vs2015_x64_md.lib", "_x64.lib"]).
#include <math.h>
#include "vec_math.h"

int v3_add(V3* out, const V3* a, const V3* b) {
    out->x = a->x + b->x;
    out->y = a->y + b->y;
    out->z = a->z + b->z;
    return 0;
}

int v3_norm(const V3* v, double* out_len) {
    double sq = (double)v->x * v->x + (double)v->y * v->y + (double)v->z * v->z;
    *out_len = sqrt(sq);
    return 0;
}
