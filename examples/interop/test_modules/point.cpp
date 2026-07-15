#include "point.h"
#include <vector>

class POINT {
public:
    int x, y;
    POINT(int x, int y) : x(x), y(y) {}
};

// Object pool: index returned by point_create() is the stable handle.
static std::vector<POINT*> g_pool;

extern "C" {

int point_create(int x, int y) {
    g_pool.push_back(new POINT(x, y));
    return static_cast<int>(g_pool.size()) - 1;
}

void point_destroy(int h) {
    if (h >= 0 && h < static_cast<int>(g_pool.size()) && g_pool[h]) {
        delete g_pool[h];
        g_pool[h] = nullptr;
    }
}

int  point_get_x(int h)        { return g_pool[h]->x; }
int  point_get_y(int h)        { return g_pool[h]->y; }
void point_set_x(int h, int v) { g_pool[h]->x = v; }
void point_set_y(int h, int v) { g_pool[h]->y = v; }

void point_move(int h, int dx, int dy) {
    g_pool[h]->x += dx;
    g_pool[h]->y += dy;
}

int point_distance_sq(int ha, int hb) {
    int dx = g_pool[ha]->x - g_pool[hb]->x;
    int dy = g_pool[ha]->y - g_pool[hb]->y;
    return dx * dx + dy * dy;
}

} // extern "C"
