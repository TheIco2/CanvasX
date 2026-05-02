// prism-runtime/src/animate/easing.rs
//
// Easing functions for animations and transitions.

use crate::prd::style::EasingFunction;

/// Evaluate an easing function at time `t` (0.0–1.0).
pub fn evaluate(easing: &EasingFunction, t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    match easing {
        EasingFunction::Linear => t,
        EasingFunction::Ease => cubic_bezier(0.25, 0.1, 0.25, 1.0, t),
        EasingFunction::EaseIn => cubic_bezier(0.42, 0.0, 1.0, 1.0, t),
        EasingFunction::EaseOut => cubic_bezier(0.0, 0.0, 0.58, 1.0, t),
        EasingFunction::EaseInOut => cubic_bezier(0.42, 0.0, 0.58, 1.0, t),
        EasingFunction::CubicBezier(x1, y1, x2, y2) => cubic_bezier(*x1, *y1, *x2, *y2, t),
    }
}

/// Cubic Bézier evaluation using Newton's method.
/// Parameters: control points (x1, y1) and (x2, y2).
/// The curve goes from (0,0) to (1,1).
fn cubic_bezier(x1: f32, y1: f32, x2: f32, y2: f32, t: f32) -> f32 {
    if t <= 0.0 { return 0.0; }
    if t >= 1.0 { return 1.0; }

    // Find the parametric t for the given x using Newton's method.
    let mut param = t; // Initial guess
    for _ in 0..8 {
        let x = sample_curve_x(x1, x2, param) - t;
        if x.abs() < 1e-6 { break; }
        let dx = sample_curve_dx(x1, x2, param);
        if dx.abs() < 1e-6 { break; }
        param -= x / dx;
    }
    param = param.clamp(0.0, 1.0);

    sample_curve_y(y1, y2, param)
}

fn sample_curve_x(x1: f32, x2: f32, t: f32) -> f32 {
    ((1.0 - 3.0 * x2 + 3.0 * x1) * t + (3.0 * x2 - 6.0 * x1)) * t + 3.0 * x1 * t
    // Expanded: ( (1-3x2+3x1)*t + (3x2-6x1) )*t + 3x1 ) * t
    // Actually: 3*(1-t)^2*t*x1 + 3*(1-t)*t^2*x2 + t^3
    // Using Horner's form for efficiency
}

fn sample_curve_y(y1: f32, y2: f32, t: f32) -> f32 {
    ((1.0 - 3.0 * y2 + 3.0 * y1) * t + (3.0 * y2 - 6.0 * y1)) * t + 3.0 * y1 * t
}

fn sample_curve_dx(x1: f32, x2: f32, t: f32) -> f32 {
    (3.0 * (1.0 - 3.0 * x2 + 3.0 * x1) * t + 2.0 * (3.0 * x2 - 6.0 * x1)) * t + 3.0 * x1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_linear() {
        assert!((evaluate(&EasingFunction::Linear, 0.5) - 0.5).abs() < 0.001);
    }

    #[test]
    fn test_ease_boundaries() {
        assert!((evaluate(&EasingFunction::Ease, 0.0)).abs() < 0.001);
        assert!((evaluate(&EasingFunction::Ease, 1.0) - 1.0).abs() < 0.001);
    }
}

