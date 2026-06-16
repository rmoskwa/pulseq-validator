//! This module could use some tests
use std::ops::Mul;

#[derive(Clone, Copy)]
pub struct Quaternion(pub [f64; 4]);

impl Default for Quaternion {
    fn default() -> Self {
        Self([1.0, 0.0, 0.0, 0.0])
    }
}

impl Mul for Quaternion {
    type Output = Self;

    fn mul(self, rhs: Self) -> Self::Output {
        let x = self.0;
        let y = rhs.0;
        Self([
            x[0] * y[0] - x[1] * y[1] - x[2] * y[2] - x[3] * y[3],
            x[0] * y[1] + x[1] * y[0] + x[2] * y[3] - x[3] * y[2],
            x[0] * y[2] - x[1] * y[3] + x[2] * y[0] + x[3] * y[1],
            x[0] * y[3] + x[1] * y[2] - x[2] * y[1] + x[3] * y[0],
        ])
    }
}

impl Quaternion {
    pub fn norm(&self) -> f64 {
        (self.0[0].powi(2) + self.0[1].powi(2) + self.0[2].powi(2) + self.0[3].powi(2)).sqrt()
    }

    pub fn is_unit(&self) -> bool {
        const EPS: f64 = 1e-9;
        (1.0 - self.norm()).abs() <= EPS
    }

    pub fn is_identity(&self) -> bool {
        self.0 == [1.0, 0.0, 0.0, 0.0]
    }
}

#[derive(Clone, Copy)]
pub struct Transform {
    pub scale: f64,
    pub rotation: Quaternion,
    /// Position will be ignored by the interpreter until I know how to apply it
    pub position: [f64; 3],
}

impl Default for Transform {
    fn default() -> Self {
        Self {
            scale: 1.0,
            rotation: Quaternion::default(),
            position: [0.0; 3],
        }
    }
}

impl Transform {
    /// converts the user defined FOV into a 3x3 transformation matrix that can
    /// be applied directly onto gradients to realize the scaling / rotation.
    /// This means inverting the scale *and the rotation!*
    pub fn to_grad_transform(self) -> [[f64; 3]; 3] {
        let q = self.rotation.0;
        let q = [q[0], -q[1], -q[2], -q[3]];
        let s = 1.0 / self.scale;
        [
            [
                s * (1.0 - 2.0 * (q[2].powi(2) + q[3].powi(2))),
                s * (2.0 * (q[1] * q[2] - q[0] * q[3])),
                s * (2.0 * (q[1] * q[3] + q[0] * q[2])),
            ],
            [
                s * (2.0 * (q[1] * q[2] + q[0] * q[3])),
                s * (1.0 - 2.0 * (q[1].powi(2) + q[3].powi(2))),
                s * (2.0 * (q[2] * q[3] - q[0] * q[1])),
            ],
            [
                s * (2.0 * (q[1] * q[3] - q[0] * q[2])),
                s * (2.0 * (q[2] * q[3] + q[0] * q[1])),
                s * (1.0 - 2.0 * (q[1].powi(2) + q[2].powi(2))),
            ],
        ]
    }

    /// Returns `true` iff the 3x3 part is a uniformly-scaled rotation /
    /// reflection — orthogonal columns with all three column norms equal.
    /// The common scale `s = ||c_i||` may be any positive finite value.
    /// Pure rotation (`s == 1`) passes; non-uniform scale, shear,
    /// non-orthogonal columns, and a zero-scale degenerate matrix all fail.
    #[allow(clippy::indexing_slicing)]
    pub fn validate(&self) -> bool {
        0.0 < self.scale
            && self.scale.is_finite()
            && self.position[0].is_finite()
            && self.position[1].is_finite()
            && self.position[2].is_finite()
            && self.rotation.is_unit()
    }
}
