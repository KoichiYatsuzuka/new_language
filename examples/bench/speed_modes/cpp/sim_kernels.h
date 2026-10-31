// sim_kernels.h - C API of the C++ kernels used by the speed study (mode 3, import[cpp-dll]).
// Keep this file ASCII-only (MSVC reads sources as CP932 by default).
// Operation order matches sim_core.ar of modes 1/2 so that the results agree bit-for-bit.
#pragma once
#include "ar_ptr_protocol.h"

typedef struct { double x, y, z; } V3;

#ifdef __cplusplus
extern "C" {
#endif

// p += v * s   (Vec3.add_scaled)
void sk_add_scaled(V3* p, const V3* v, double s);
// reflect vel on each axis where |pos| > lim; returns the number of reflections (Body.bounce)
int sk_bounce(const V3* pos, V3* vel, double lim);
// 0.5 * mass * dot(vel, vel)   (Body.kinetic)
double sk_kinetic(double mass, const V3* vel);
// sum of a "float64" array (Body.trail_sum); -1.0 for another element type
double sk_sum_f64(const ArPtrProtocol* xs);
// name + "#" + hits, in a per-thread buffer that the next call overwrites (Body.label)
const char* sk_label(const char* name, int hits);
// V3 by value (used by run_spawn)
V3 sk_make(double x, double y, double z);

#ifdef __cplusplus
}
#endif
