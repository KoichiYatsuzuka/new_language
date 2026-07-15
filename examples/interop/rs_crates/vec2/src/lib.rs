/// 2D vector with f64 components.
pub struct Vec2 {
    pub x: f64,
    pub y: f64,
}

impl Vec2 {
    pub fn new(x: f64, y: f64) -> Self {
        Vec2 { x, y }
    }

    /// Euclidean length.
    pub fn length(&self) -> f64 {
        (self.x * self.x + self.y * self.y).sqrt()
    }

    /// Dot product with the vector (ox, oy).
    pub fn dot(&self, ox: f64, oy: f64) -> f64 {
        self.x * ox + self.y * oy
    }

    /// Scale in-place by `factor`.
    pub fn scale(&mut self, factor: f64) {
        self.x *= factor;
        self.y *= factor;
    }

    /// Squared Euclidean length (cheaper than length when only comparison is needed).
    pub fn length_sq(&self) -> f64 {
        self.x * self.x + self.y * self.y
    }

    /// Normalize to unit length.  Returns (0, 0) for the zero vector.
    pub fn normalize(&mut self) {
        let len = self.length();
        if len > 0.0 {
            self.x /= len;
            self.y /= len;
        }
    }

    /// Return a new Vec2 that is the component-wise sum of self and (ox, oy).
    pub fn add(&self, ox: f64, oy: f64) -> Vec2 {
        Vec2 { x: self.x + ox, y: self.y + oy }
    }

    /// Return a new Vec2 scaled by `factor` (self is unchanged).
    pub fn scaled(&self, factor: f64) -> Vec2 {
        Vec2 { x: self.x * factor, y: self.y * factor }
    }
}

/// Distance between two points.
pub fn distance(x1: f64, y1: f64, x2: f64, y2: f64) -> f64 {
    let dx = x2 - x1;
    let dy = y2 - y1;
    (dx * dx + dy * dy).sqrt()
}
