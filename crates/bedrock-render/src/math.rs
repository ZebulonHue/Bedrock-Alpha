//! Orbit camera and minimal 4×4 matrix math (column-major, wgpu NDC).

/// Orbit camera: a target point plus yaw/pitch/distance around it.
#[derive(Debug, Clone, Copy)]
pub struct Camera {
    /// The point the camera looks at (world coordinates).
    pub target: [f32; 3],
    /// Rotation around the vertical axis, radians.
    pub yaw: f32,
    /// Elevation above the horizon, radians (kept slightly above zero so the
    /// camera never dips under the world).
    pub pitch: f32,
    /// Distance from the target, in blocks.
    pub distance: f32,
    /// Multiplier on fly speed, driven by the scroll wheel.
    ///
    /// Fly speed is otherwise tied to `distance`, which makes close-up work
    /// crawl and long traverses slow, with no way to override it. Clamped so
    /// the camera can neither stop dead nor leave the world in one frame.
    pub speed_scale: f32,
}

/// Slowest and fastest the scroll wheel may make the fly camera.
const MIN_SPEED_SCALE: f32 = 0.05;
const MAX_SPEED_SCALE: f32 = 20.0;

impl Default for Camera {
    fn default() -> Self {
        Self {
            target: [8.0, 70.0, 8.0],
            yaw: 0.8,
            pitch: 0.6,
            distance: 140.0,
            speed_scale: 1.0,
        }
    }
}

impl Camera {
    /// Minimum elevation above the horizon.
    const MIN_PITCH: f32 = 0.05;
    /// Maximum elevation (just under straight down).
    const MAX_PITCH: f32 = 1.52;

    /// Orbit by screen-space drag deltas.
    pub fn orbit(&mut self, dx: f32, dy: f32) {
        self.yaw += dx * 0.01;
        self.pitch = (self.pitch + dy * 0.01).clamp(Self::MIN_PITCH, Self::MAX_PITCH);
    }

    /// Pan the target in the camera plane by screen-space drag deltas.
    pub fn pan(&mut self, dx: f32, dy: f32) {
        let right = [self.yaw.cos(), 0.0, -self.yaw.sin()];
        let scale = self.distance * 0.002;
        for (axis, target) in self.target.iter_mut().enumerate() {
            *target -= right[axis] * dx * scale;
        }
        self.target[1] += dy * scale;
    }

    /// Change fly speed by a scroll delta: wheel up faster, wheel down slower.
    ///
    /// Multiplicative rather than additive so each notch is the same
    /// proportional change whether you are crawling or sprinting.
    pub fn adjust_speed(&mut self, scroll: f32) {
        let factor = (scroll * 0.004).exp();
        self.speed_scale = (self.speed_scale * factor).clamp(MIN_SPEED_SCALE, MAX_SPEED_SCALE);
    }

    /// Zoom by a scroll delta (positive scrolls in).
    pub fn zoom(&mut self, scroll: f32) {
        self.distance = (self.distance * (1.0 - scroll * 0.001)).clamp(8.0, 2000.0);
    }

    /// Fly the camera using WASD-style input. `forward/back/left/right` are
    /// −1.0/0.0/1.0, `up/down` are −1.0/0.0/1.0. Movement speed scales with
    /// distance so zoomed-out views travel faster.
    pub fn fly(&mut self, forward: f32, right: f32, up: f32) {
        let speed = self.distance * 0.03 * self.speed_scale;
        let (sin_yaw, cos_yaw) = self.yaw.sin_cos();
        // Forward = direction from eye toward target (negated sin/cos from
        // eye()).
        let fx = -sin_yaw;
        let fz = -cos_yaw;
        // Right = forward turned a quarter turn clockwise seen from above.
        // Check it at yaw 0: forward is (0, -1), facing north, so right must
        // be (1, 0) -- east. The previous signs gave west, which swapped A
        // and D for every heading.
        let rx = cos_yaw;
        let rz = -sin_yaw;
        self.target[0] += (fx * forward + rx * right) * speed;
        self.target[2] += (fz * forward + rz * right) * speed;
        self.target[1] += up * speed;
    }

    /// Eye position in world coordinates.
    pub fn eye(&self) -> [f32; 3] {
        let (sin_yaw, cos_yaw) = self.yaw.sin_cos();
        let (sin_pitch, cos_pitch) = self.pitch.sin_cos();
        [
            self.target[0] + self.distance * cos_pitch * sin_yaw,
            self.target[1] + self.distance * sin_pitch,
            self.target[2] + self.distance * cos_pitch * cos_yaw,
        ]
    }

    /// Combined view-projection matrix (column-major) for the given aspect.
    pub fn view_proj(&self, aspect: f32) -> [[f32; 4]; 4] {
        let view = look_at(self.eye(), self.target, [0.0, 1.0, 0.0]);
        // near = 1.0 (was 0.5): tighter near plane improves depth precision.
        // far  = 6000.0: covers the largest Minecraft worlds comfortably.
        // Reverse-Z projection: depth compare must be Greater, clear to 0.0.
        let proj = perspective(60.0_f32.to_radians(), aspect, 1.0, 6000.0);
        mat_mul(&proj, &view)
    }
}

/// Right-handed **Reverse-Z** perspective projection for wgpu's `[0, 1]` depth
/// range. Near maps to depth `1.0`, far maps to depth `0.0`.
///
/// This distributes depth-buffer precision where it matters for large voxel
/// worlds: at *distance* rather than near the camera. Combined with a
/// `CompareFunction::Greater` depth test and a clear value of `0.0` it
/// eliminates the black-spot z-fighting artifacts visible on distant terrain.
fn perspective(fov_y: f32, aspect: f32, near: f32, far: f32) -> [[f32; 4]; 4] {
    let f = 1.0 / (fov_y * 0.5).tan();
    let aspect = aspect.max(1e-4);
    [
        [f / aspect, 0.0, 0.0, 0.0],
        [0.0, f, 0.0, 0.0],
        [0.0, 0.0, near / (far - near), -1.0],
        [0.0, 0.0, near * far / (far - near), 0.0],
    ]
}

/// Right-handed look-at view matrix.
fn look_at(eye: [f32; 3], center: [f32; 3], up: [f32; 3]) -> [[f32; 4]; 4] {
    let f = normalize(sub(center, eye));
    let s = normalize(cross(f, up));
    let u = cross(s, f);
    [
        [s[0], u[0], -f[0], 0.0],
        [s[1], u[1], -f[1], 0.0],
        [s[2], u[2], -f[2], 0.0],
        [-dot(s, eye), -dot(u, eye), dot(f, eye), 1.0],
    ]
}

/// Column-major matrix product `a * b`.
fn mat_mul(a: &[[f32; 4]; 4], b: &[[f32; 4]; 4]) -> [[f32; 4]; 4] {
    let mut out = [[0.0; 4]; 4];
    for (col, out_col) in out.iter_mut().enumerate() {
        for row in 0..4 {
            out_col[row] = (0..4).map(|k| a[k][row] * b[col][k]).sum();
        }
    }
    out
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn normalize(v: [f32; 3]) -> [f32; 3] {
    let len = dot(v, v).sqrt().max(1e-8);
    [v[0] / len, v[1] / len, v[2] / len]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_projects_to_clip_center() {
        let camera = Camera::default();
        let mvp = camera.view_proj(16.0 / 9.0);
        let p = camera.target;
        // Column-major multiply: clip = mvp * [p, 1].
        let mut clip = [0.0f32; 4];
        for (col, m_col) in mvp.iter().enumerate() {
            let component = [p[0], p[1], p[2], 1.0][col];
            for row in 0..4 {
                clip[row] += m_col[row] * component;
            }
        }
        assert!(clip[3] > 0.0, "target must be in front of the camera");
        assert!(clip[0].abs() < 1e-3 && clip[1].abs() < 1e-3);
        // Reverse-Z: near→1.0, far→0.0, so target (at mid-distance) is in
        // (0.0, 1.0) but closer to 0.0 than to 1.0.
        let ndc_z = clip[2] / clip[3];
        assert!((0.0..=1.0).contains(&ndc_z), "ndc z {ndc_z} out of range");
    }

    /// D must move east when facing north, not west. The two directions differ
    /// only in sign, so an inverted right vector is invisible in isolation and
    /// only shows up as the controls being mirrored.
    #[test]
    fn strafe_right_goes_right() {
        let mut camera = Camera {
            yaw: 0.0,
            target: [0.0, 0.0, 0.0],
            ..Camera::default()
        };
        camera.fly(0.0, 1.0, 0.0);
        assert!(camera.target[0] > 0.0, "facing north, D must move east (+X)");
        assert!(camera.target[2].abs() < 1e-3, "strafing must not move along Z");

        // Facing west (yaw = 90 degrees), right is north (-Z).
        let mut camera = Camera {
            yaw: std::f32::consts::FRAC_PI_2,
            target: [0.0, 0.0, 0.0],
            ..Camera::default()
        };
        camera.fly(0.0, 1.0, 0.0);
        assert!(camera.target[2] < 0.0, "facing west, D must move north (-Z)");
    }

    /// Wheel up speeds the camera up, wheel down slows it, and neither can run
    /// away: at the limits the camera must still be able to move, and must not
    /// cross the world in a frame.
    #[test]
    fn scroll_changes_speed_within_limits() {
        let mut camera = Camera::default();
        let base = camera.speed_scale;
        camera.adjust_speed(120.0);
        assert!(camera.speed_scale > base, "wheel up must speed up");
        camera.adjust_speed(-240.0);
        assert!(camera.speed_scale < base, "wheel down must slow down");
        for _ in 0..200 {
            camera.adjust_speed(-1000.0);
        }
        assert_eq!(camera.speed_scale, MIN_SPEED_SCALE);
        for _ in 0..200 {
            camera.adjust_speed(1000.0);
        }
        assert_eq!(camera.speed_scale, MAX_SPEED_SCALE);
    }

    #[test]
    fn zoom_is_clamped() {
        let mut camera = Camera::default();
        camera.zoom(100_000.0);
        assert_eq!(camera.distance, 8.0);
        for _ in 0..3 {
            camera.zoom(-100_000.0);
        }
        assert_eq!(camera.distance, 2000.0);
    }
}
