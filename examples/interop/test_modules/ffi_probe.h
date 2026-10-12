// ffi_probe.h - C API for checking how values cross the FFI boundary
// (implementation_plans/ffi_value_exchange_plan.md, task 1-1).
//
// Both artifacts are built from ffi_probe.c by build_ffi_probe.ps1:
//   ffi_probe.dll      ... import[cpp-dll] test_modules.ffi_probe
//   ffi_probe_x64.lib  ... import[cpp-lib] test_modules.ffi_probe
//
// Some functions are NOT handled correctly by the current bridge (plan section 0.1);
// the examples that use them are added by the task that fixes each case.
//
// NOTE: keep this file ASCII-only. MSVC reads sources as CP932 by default, and the
// cpp-lib shim #includes this header; a UTF-8 comment can swallow the next line.
#pragma once

typedef struct { int id; double score; } Rec;        // 16 bytes (returned via hidden pointer on x64)
typedef struct { int x; int y; } Pair;               // 8 bytes (returned in RAX on x64)
typedef struct { float x, y, z; } Vec3f;             // 12 bytes
typedef struct { int id; const char* name; } Named;  // has a pointer field
typedef struct { unsigned char r, g, b, a; } Rgba8;  // 4 bytes: fields narrower than int (task 1-3)
typedef struct Db Db;                                // opaque (forward declaration only)

#ifdef __cplusplus
extern "C" {
#endif

// -- strings -------------------------------------------------------------------
const char* fp_greet(void);                 // "hello from C" (static string)
const char* fp_echo(const char* s);         // returns the pointer it received
const char* fp_utf8(void);                  // "日本語" in UTF-8 (9 bytes)
const char* fp_null(void);                  // NULL
int fp_len(const char* s);                  // byte length, -1 for NULL
void fp_fill(char* buf, int n);             // writes "filled" into buf (output buffer)
const wchar_t* fp_wide(void);               // L"wide"
int fp_wlen(const wchar_t* s);              // number of wchar_t, -1 for NULL

// -- structs (by value / by pointer) ------------------------------------------
Rec fp_make_rec(int id, double score);
double fp_rec_score_val(Rec r);
Pair fp_make_pair(int x, int y);
int fp_pair_sum(Pair p);
Vec3f fp_make_vec3(float x, float y, float z);
float fp_vec3_sum(Vec3f v);
double fp_rec_score(const Rec* r);          // -1.0 for NULL
Rec* fp_rec_static(void);                   // points at a static Rec inside the library
Named fp_make_named(int id);                // name = "named-<id>" (static buffer)
int fp_named_len(Named n);                  // strlen(n.name), -1 for NULL
Rgba8 fp_make_rgba(int r, int g, int b, int a);
int fp_rgba_sum(Rgba8 c);                   // r + g + b + a

// -- pointers (handles) --------------------------------------------------------
void* fp_handle_new(void);
int fp_handle_get(void* h);                 // 4242 for the value of fp_handle_new, else -1
Db* fp_db_open(const char* name);
int fp_db_count(Db* db);
void fp_db_close(Db* db);

// -- arrays --------------------------------------------------------------------
int fp_sum(const int* xs, int n);

#ifdef __cplusplus
}
#endif
