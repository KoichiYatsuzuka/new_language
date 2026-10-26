// sim_kernels.cpp - C++ implementation of sim_kernels.h (built into sim_kernels.dll by build_kernels.ps1).
#include <cstring>
#include <string>
#include "sim_kernels.h"

extern "C" {

void sk_add_scaled(V3* p, const V3* v, double s) {
    p->x = p->x + v->x * s;
    p->y = p->y + v->y * s;
    p->z = p->z + v->z * s;
}

int sk_bounce(const V3* pos, V3* vel, double lim) {
    int n = 0;
    if (pos->x > lim || pos->x < 0.0 - lim) { vel->x = 0.0 - vel->x; n += 1; }
    if (pos->y > lim || pos->y < 0.0 - lim) { vel->y = 0.0 - vel->y; n += 1; }
    if (pos->z > lim || pos->z < 0.0 - lim) { vel->z = 0.0 - vel->z; n += 1; }
    return n;
}

double sk_kinetic(double mass, const V3* vel) {
    return 0.5 * mass * (vel->x * vel->x + vel->y * vel->y + vel->z * vel->z);
}

double sk_sum_f64(const ArPtrProtocol* xs) {
    if (!xs || !xs->elem_type || std::strcmp(xs->elem_type, "float64") != 0) return -1.0;
    const double* a = reinterpret_cast<const double*>(static_cast<uintptr_t>(xs->address));
    double s = 0.0;
    for (int64_t i = 0; i < xs->length; i++) s += a[i];
    return s;
}

const char* sk_label(const char* name, int hits) {
    thread_local std::string buf;
    buf.assign(name ? name : "");
    buf.push_back('#');
    buf += std::to_string(hits);
    return buf.c_str();
}

V3 sk_make(double x, double y, double z) {
    V3 v;
    v.x = x;
    v.y = y;
    v.z = z;
    return v;
}

}  // extern "C"
