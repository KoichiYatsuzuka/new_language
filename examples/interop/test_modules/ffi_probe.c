// ffi_probe.c - implementation of ffi_probe.h (built by build_ffi_probe.ps1). Keep ASCII-only.
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <wchar.h>
#include "ffi_probe.h"

struct Db { int count; char name[32]; };

static char g_named[64];
static Rec g_rec = { 7, 2.5 };
static int g_handle_payload = 4242;

const char* fp_greet(void) { return "hello from C"; }
const char* fp_echo(const char* s) { return s; }
const char* fp_utf8(void) { return "\xE6\x97\xA5\xE6\x9C\xAC\xE8\xAA\x9E"; }
const char* fp_null(void) { return NULL; }
int fp_len(const char* s) { return s ? (int)strlen(s) : -1; }
void fp_fill(char* buf, int n) {
    if (buf && n > 0) { strncpy(buf, "filled", (size_t)n - 1); buf[n - 1] = 0; }
}
const wchar_t* fp_wide(void) { return L"wide"; }
int fp_wlen(const wchar_t* s) { return s ? (int)wcslen(s) : -1; }

Rec fp_make_rec(int id, double score) { Rec r; r.id = id; r.score = score; return r; }
double fp_rec_score_val(Rec r) { return r.score; }
Pair fp_make_pair(int x, int y) { Pair p; p.x = x; p.y = y; return p; }
int fp_pair_sum(Pair p) { return p.x + p.y; }
Vec3f fp_make_vec3(float x, float y, float z) { Vec3f v; v.x = x; v.y = y; v.z = z; return v; }
float fp_vec3_sum(Vec3f v) { return v.x + v.y + v.z; }
double fp_rec_score(const Rec* r) { return r ? r->score : -1.0; }
Rec* fp_rec_static(void) { return &g_rec; }
Named fp_make_named(int id) {
    Named n;
    snprintf(g_named, sizeof(g_named), "named-%d", id);
    n.id = id;
    n.name = g_named;
    return n;
}
int fp_named_len(Named n) { return n.name ? (int)strlen(n.name) : -1; }
Rgba8 fp_make_rgba(int r, int g, int b, int a) {
    Rgba8 c;
    c.r = (unsigned char)r; c.g = (unsigned char)g; c.b = (unsigned char)b; c.a = (unsigned char)a;
    return c;
}
int fp_rgba_sum(Rgba8 c) { return c.r + c.g + c.b + c.a; }
Mat2 fp_make_mat2(void) { Mat2 m; m.m[0] = 1.0f; m.m[1] = 0.0f; m.m[2] = 0.0f; m.m[3] = 1.0f; return m; }
float fp_mat2_trace(Mat2 m) { return m.m[0] + m.m[3]; }

unsigned int fp_u32_max(void) { return 4294967295u; }
int fp_u32_is_big(unsigned int v) { return v > 2147483647u ? 1 : 0; }
unsigned char fp_u8_add(unsigned char a, unsigned char b) { return (unsigned char)(a + b); }
signed char fp_i8_neg(signed char v) { return (signed char)(-v); }
short fp_i16_min(void) { return (short)-32768; }
unsigned short fp_u16_max(void) { return 65535u; }
long fp_long_neg(void) { return -5L; }
long long fp_i64_min(void) { return (-9223372036854775807LL - 1); }
unsigned long long fp_u64_small(void) { return 42ull; }
unsigned long long fp_u64_big(void) { return 18446744073709551615ull; }
bool fp_is_even(int v) { return (v % 2) == 0; }
void fp_u8_out(unsigned char* out) { if (out) *out = 250; }
void fp_i16_out(short* out) { if (out) *out = -2; }
void fp_long_out(long* out) { if (out) *out = -7L; }
void fp_u64_out(unsigned long long* out, int big) { if (out) *out = big ? 18446744073709551615ull : 7ull; }
U64Box fp_make_u64box(int big, int tag) { U64Box b; b.v = big ? 18446744073709551615ull : 9ull; b.tag = tag; return b; }

void* fp_handle_new(void) { return &g_handle_payload; }
int fp_handle_get(void* h) { return h == (void*)&g_handle_payload ? g_handle_payload : -1; }
Db* fp_db_open(const char* name) {
    Db* db = (Db*)calloc(1, sizeof(Db));
    if (db && name) { strncpy(db->name, name, sizeof(db->name) - 1); db->count = (int)strlen(db->name); }
    return db;
}
int fp_db_count(Db* db) { return db ? db->count : -1; }
void fp_db_close(Db* db) { free(db); }

int fp_sum(const int* xs, int n) { int s = 0; for (int i = 0; i < n; i++) s += xs[i]; return s; }

static int g_calls = 0;
int fp_count_int(int v) { g_calls++; return v; }
int fp_count_flag(bool b) { g_calls++; return b ? 1 : 0; }
int fp_count_double(double d) { g_calls++; return (int)d; }
int fp_call_count(void) { return g_calls; }

static int g_released = 0;
static void fp_release_squares(void* ctx, uint64_t address) { (void)ctx; g_released++; free((void*)(uintptr_t)address); }
ArPtrProtocol fp_squares(int n) {
    ArPtrProtocol p = { 0, "int32", 0, 0, 0, fp_release_squares, NULL };
    if (n <= 0) return p;
    int32_t* xs = (int32_t*)malloc(sizeof(int32_t) * (size_t)n);
    if (!xs) return p;
    for (int i = 0; i < n; i++) xs[i] = i * i;
    p.address = (uint64_t)(uintptr_t)xs;
    p.length = n;
    return p;
}
static const char g_static_text[] = "hello";
ArPtrProtocol fp_static_text(void) {
    ArPtrProtocol p = { (uint64_t)(uintptr_t)g_static_text, "uint8", AR_PTR_STATIC | AR_PTR_NUL_TERMINATED | AR_PTR_CONST,
                        0, -1, NULL, NULL };
    return p;
}
ArPtrProtocol fp_no_release(void) {
    ArPtrProtocol p = { (uint64_t)(uintptr_t)g_static_text, "uint8", 0, 0, 5, NULL, NULL };
    return p;
}
int fp_released(void) { return g_released; }
int fp_protocol_sum(const ArPtrProtocol* p) {
    if (!p || !p->elem_type || strcmp(p->elem_type, "int32") != 0) return -1;
    const int32_t* xs = (const int32_t*)(uintptr_t)p->address;
    int s = 0;
    for (int64_t i = 0; i < p->length; i++) s += xs[i];
    return s;
}
const char* fp_protocol_type(ArPtrProtocol p) { return p.elem_type ? p.elem_type : "(null)"; }
int fp_rec_label(const Rec* r, const char* label) { return (r ? r->id : -1000) + (label ? (int)strlen(label) : -1); }
void fp_protocol_free(ArPtrProtocol* p) {
    if (!p || p->address == 0) return;
    if (p->release) p->release(p->release_ctx, p->address);
    p->address = 0;
}
