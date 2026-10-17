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
#include <stdbool.h>
#include "../../../bridge/ptr_protocol/ar_ptr_protocol.h"
#include "ffi_probe_extra.h"

// Default arguments written the way DxLib writes them (task 7-13): C++ sees `= x`, C sees nothing.
#ifndef DEFAULTPARAM
#ifdef __cplusplus
#define DEFAULTPARAM(x) x
#else
#define DEFAULTPARAM(x)
#endif
#endif
#define FP_MODE_DEFAULT 3

typedef struct { int id; double score; } Rec;        // 16 bytes (returned via hidden pointer on x64)
typedef struct { int x; int y; } Pair;               // 8 bytes (returned in RAX on x64)
typedef struct { float x, y, z; } Vec3f;             // 12 bytes
typedef struct { int id; const char* name; } Named;  // has a pointer field
typedef struct { unsigned char r, g, b, a; } Rgba8;  // 4 bytes: fields narrower than int (tasks 1-3, 1-6)
typedef struct { float m[4]; } Mat2;                 // has an array field: its layout is not known exactly
typedef struct { unsigned long long v; int tag; } U64Box;  // 16 bytes: an unsigned 64-bit field (task 1-6)
typedef struct Db Db;                                // opaque (forward declaration only)

#ifdef __cplusplus
extern "C" {
#endif

// -- strings -------------------------------------------------------------------
const char* fp_greet(void);                 // "hello from C" (static string)
const char* fp_echo(const char* s);         // returns the pointer it received
const char* fp_utf8(void);                  // Japanese "nihongo" (3 characters) in UTF-8 (9 bytes)
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
Mat2 fp_make_mat2(void);
float fp_mat2_trace(Mat2 m);

// -- integer widths and signedness (task 1-6) ------------------------------------
unsigned int fp_u32_max(void);              // 4294967295
int fp_u32_is_big(unsigned int v);          // 1 if v > 2147483647
unsigned char fp_u8_add(unsigned char a, unsigned char b);  // (a + b) mod 256
signed char fp_i8_neg(signed char v);       // -v
short fp_i16_min(void);                     // -32768
unsigned short fp_u16_max(void);            // 65535
long fp_long_neg(void);                     // -5 (long is 4 bytes on Windows)
long long fp_i64_min(void);                 // -9223372036854775808
unsigned long long fp_u64_small(void);      // 42
unsigned long long fp_u64_big(void);        // 18446744073709551615 (does not fit in a signed 64-bit int)
bool fp_is_even(int v);
void fp_u8_out(unsigned char* out);         // *out = 250
void fp_i16_out(short* out);                // *out = -2
void fp_long_out(long* out);                // *out = -7
void fp_u64_out(unsigned long long* out, int big);  // *out = big ? 2^64-1 : 7
U64Box fp_make_u64box(int big, int tag);    // v = big ? 2^64-1 : 9

// -- pointers (handles) --------------------------------------------------------
void* fp_handle_new(void);
int fp_handle_get(void* h);                 // 4242 for the value of fp_handle_new, else -1
Db* fp_db_open(const char* name);
int fp_db_count(Db* db);
void fp_db_close(Db* db);

// -- arrays --------------------------------------------------------------------
int fp_sum(const int* xs, int n);

// -- argument checks (task 7-12): each fp_count_* call is counted, so a test can see whether it was called -----
int fp_count_int(int v);                    // returns v
int fp_count_flag(bool b);                  // returns 1 for true, 0 for false
int fp_count_double(double d);              // returns (int)d
int fp_call_count(void);                    // number of fp_count_* calls so far

// -- PtrProtocol (tasks 4-2 ... 4-7) -------------------------------------------------
ArPtrProtocol fp_squares(int n);             // int32 squares 0..n-1, owned: Arrow releases it (n <= 0: NULL)
ArPtrProtocol fp_static_text(void);          // "hello" as uint8, STATIC and NUL-terminated (length -1)
ArPtrProtocol fp_no_release(void);           // invalid on purpose: owned memory without a release function
int fp_released(void);                       // how many fp_squares buffers have been released
int fp_protocol_sum(const ArPtrProtocol* p); // sum of int32 elements (-1 if the element type is not int32)
void fp_protocol_free(ArPtrProtocol* p);     // frees a fp_squares buffer and writes address = 0
const char* fp_protocol_type(ArPtrProtocol p); // the element type name it received ("(null)" for NULL)
int fp_rec_label(const Rec* r, const char* label); // r->id + strlen(label): a struct pointer and a C string together (4-4)

// -- structs with pointer fields and arrays of structs (task 4-6) ------------------------------
typedef struct { int tag; ArPtrProtocol data; } Tagged;  // a struct that owns an int32 array
Tagged fp_tagged(int n);                     // tag = n, data = fp_squares(n)
int fp_tagged_sum(Tagged t);                 // tag + the sum of data (passes the struct back by value)
ArPtrProtocol fp_pairs(int n);               // Pair[n] = {i, 2*i}, owned (released by Arrow)
ArPtrProtocol fp_named_list(void);           // Named[2] (pointer fields), STATIC

// -- Arrow lists / bytes passed as ArPtrProtocol (task 4-7) -------------------------------------
long long fp_sum64(const ArPtrProtocol* p);  // sum of "int64" elements (-1 for another element type)
void fp_double64(ArPtrProtocol* p);          // doubles each "int64" element in place
double fp_sumf(const ArPtrProtocol* p);      // sum of "float64" elements (-1.0 for another element type)
int fp_pairs_total(const ArPtrProtocol* p);  // sum of x + y over "Pair" elements (-1 for another element type)
int fp_bytes_sum(ArPtrProtocol p);           // sum of "uint8" elements, by value (-1 for another element type)

// -- arrays of structs with pointer fields, expanded by Ptr[T].to_list() (task 7-14) -------------
// Mixed: padding after `kind` and at the end; a wide string, a struct pointer, a void pointer and a bool
typedef struct { short kind; const wchar_t* label; Rec* rec; void* extra; bool flag; } Mixed;
ArPtrProtocol fp_mixed_list(void);           // Mixed[3] (some pointer fields NULL), STATIC
ArPtrProtocol fp_tagged_list(void);          // Tagged[2] (ArPtrProtocol fields, STATIC data), STATIC: to_list() refuses it

// -- default arguments (task 7-13) ----------------------------------------------------------------
// a + b + c + (s ? strlen(s) : 100): omitted arguments must be the declared defaults, not 0 / NULL
double fp_defaults(int a, int b DEFAULTPARAM( = -1 ), double c DEFAULTPARAM( = 0.5 ), const char* s DEFAULTPARAM( = NULL ));
int fp_mode(int a, int mode DEFAULTPARAM( = FP_MODE_DEFAULT ));  // a + mode; a macro default cannot be read, so `mode` is required

#ifdef __cplusplus
}
#endif
