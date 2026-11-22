// vec_math.h — minimal C API used by the P5 static-rejection example.
//
// V3 is a plain (standard-layout) struct: all fields are `float`, so the
// cpp bridge auto-generates an Arrow class with a C ABI raw layout
// (offsets 0, 4, 8).  See .claude/skills/c-abi-interop/SKILL.md (P2/P3/P5).
#pragma once

typedef struct { float x, y, z; } V3;

#ifdef __cplusplus
extern "C" {
#endif

// `out` is a mutable pointer (V3*) — the function writes the result into it,
// so callers must pass a `mut` variable.  `a` / `b` are read-only (const V3*).
int v3_add(V3* out, const V3* a, const V3* b);

// `out_len` is a mutable primitive pointer (double*) — the type-check stub
// annotates it with the pointee type (`float`), so callers must pass a
// `mut` float variable (not an int, not a `let`).
int v3_norm(const V3* v, double* out_len);

#ifdef __cplusplus
}
#endif
