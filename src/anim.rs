//! A damped-spring integrator matching SwiftUI's
//! `spring(response:dampingFraction:)`.
//!
//! SwiftUI parameterizes a spring by `response` (the natural period, i.e.
//! `2*pi/omega`) and `dampingFraction` (zeta). We integrate the standard
//! second-order ODE
//!
//! ```text
//! x'' = -omega^2 * (x - target) - 2*zeta*omega * x'
//! ```
//!
//! with a stable semi-implicit (symplectic) Euler step, substepping so the
//! result is identical regardless of frame rate.

#[derive(Clone, Copy)]
pub struct Spring {
    pub value: f32,
    pub velocity: f32,
    pub target: f32,
    omega: f32,
    zeta: f32,
}

impl Spring {
    /// `response` in seconds, `damping` is the damping fraction (zeta).
    pub fn new(value: f32, response: f32, damping: f32) -> Self {
        Self {
            value,
            velocity: 0.0,
            target: value,
            omega: std::f32::consts::TAU / response.max(0.0001),
            zeta: damping,
        }
    }

    pub fn set_target(&mut self, target: f32) {
        self.target = target;
    }

    /// Advance by `dt` seconds. Returns true while still settling.
    pub fn update(&mut self, dt: f32) -> bool {
        // Substep at a fixed 240 Hz for integrator stability at any frame rate.
        const STEP: f32 = 1.0 / 240.0;
        let mut remaining = dt.clamp(0.0, 0.1);
        while remaining > 0.0 {
            let h = remaining.min(STEP);
            let accel = -self.omega * self.omega * (self.value - self.target)
                - 2.0 * self.zeta * self.omega * self.velocity;
            self.velocity += accel * h;
            self.value += self.velocity * h;
            remaining -= h;
        }
        !self.is_settled()
    }

    pub fn is_settled(&self) -> bool {
        (self.value - self.target).abs() < 0.05 && self.velocity.abs() < 0.05
    }
}
