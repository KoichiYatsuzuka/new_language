// vec_math.h — minimal C API used by the P5 static-rejection example.
//
// V3 is a plain (standard-layout) struct: all fields are `float`, so the
// cpp bridge auto-generates an Arrow class with a C ABI raw layout
// (offsets 0, 4, 8).  See for_claude/c_abi_interop.md (P2/P3/P5).
#pragma once

typedef struct { float x, y, z; } V3;

#ifdef __cplusplus
extern "C" {
#endif

// `out` is a mutable pointer (V3*) — the function writes the result into it,
// so callers must pass a `mut` variable.  `a` / `b` are read-only (const V3*).
int v3_add(V3* out, const V3* a, const V3* b);

#ifdef __cplusplus
}
#endif
