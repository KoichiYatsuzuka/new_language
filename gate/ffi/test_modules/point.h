#pragma once

// C-compatible API for the POINT class defined in point.cpp.
// POINT is a plain struct: the bridge auto-generates a tl class for it.
// Functions use int handles returned by point_create() for C++-side operations.

class POINT {
public:
    int x;
    int y;
    POINT(int x, int y) : x(x), y(y) {}
};

#ifdef __cplusplus
extern "C" {
#endif

int  point_create(int x, int y);
void point_destroy(int handle);
int  point_get_x(int handle);
int  point_get_y(int handle);
void point_set_x(int handle, int v);
void point_set_y(int handle, int v);
void point_move(int handle, int dx, int dy);
int  point_distance_sq(int ha, int hb);

#ifdef __cplusplus
}
#endif
