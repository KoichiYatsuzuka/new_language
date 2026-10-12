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
