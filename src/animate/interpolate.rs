// canvasx-runtime/src/animate/interpolate.rs
//
// Property interpolation for animated values.

use crate::cxrd::animation::AnimatableProperty;

/// Linearly interpolate between two animatable property values.
/// Returns the interpolated property at factor `t` (0.0–1.0).
pub fn interpolate(from: &AnimatableProperty, to: &AnimatableProperty, t: f32) -> AnimatableProperty {
    match (from, to) {
        (AnimatableProperty::Opacity(a), AnimatableProperty::Opacity(b)) => {
            AnimatableProperty::Opacity(lerp(*a, *b, t))
        }
        (AnimatableProperty::TranslateX(a), AnimatableProperty::TranslateX(b)) => {
            AnimatableProperty::TranslateX(lerp(*a, *b, t))
        }
        (AnimatableProperty::TranslateY(a), AnimatableProperty::TranslateY(b)) => {
            AnimatableProperty::TranslateY(lerp(*a, *b, t))
        }
        (AnimatableProperty::ScaleX(a), AnimatableProperty::ScaleX(b)) => {
            AnimatableProperty::ScaleX(lerp(*a, *b, t))
        }
        (AnimatableProperty::ScaleY(a), AnimatableProperty::ScaleY(b)) => {
            AnimatableProperty::ScaleY(lerp(*a, *b, t))
        }
        (AnimatableProperty::Rotate(a), AnimatableProperty::Rotate(b)) => {
            AnimatableProperty::Rotate(lerp(*a, *b, t))
        }
        (AnimatableProperty::BackgroundColor(a), AnimatableProperty::BackgroundColor(b)) => {
            AnimatableProperty::BackgroundColor(a.lerp(b, t))
        }
        (AnimatableProperty::Color(a), AnimatableProperty::Color(b)) => {
            AnimatableProperty::Color(a.lerp(b, t))
        }
        (AnimatableProperty::BorderColor(a), AnimatableProperty::BorderColor(b)) => {
            AnimatableProperty::BorderColor(a.lerp(b, t))
        }
        (AnimatableProperty::BorderRadius(a), AnimatableProperty::BorderRadius(b)) => {
            AnimatableProperty::BorderRadius(lerp(*a, *b, t))
        }
        (AnimatableProperty::Width(a), AnimatableProperty::Width(b)) => {
            AnimatableProperty::Width(lerp(*a, *b, t))
        }
        (AnimatableProperty::Height(a), AnimatableProperty::Height(b)) => {
            AnimatableProperty::Height(lerp(*a, *b, t))
        }
        (AnimatableProperty::FontSize(a), AnimatableProperty::FontSize(b)) => {
            AnimatableProperty::FontSize(lerp(*a, *b, t))
        }
        (AnimatableProperty::Gap(a), AnimatableProperty::Gap(b)) => {
            AnimatableProperty::Gap(lerp(*a, *b, t))
        }
        (AnimatableProperty::PaddingTop(a), AnimatableProperty::PaddingTop(b)) => {
            AnimatableProperty::PaddingTop(lerp(*a, *b, t))
        }
        (AnimatableProperty::PaddingRight(a), AnimatableProperty::PaddingRight(b)) => {
            AnimatableProperty::PaddingRight(lerp(*a, *b, t))
        }
        (AnimatableProperty::PaddingBottom(a), AnimatableProperty::PaddingBottom(b)) => {
            AnimatableProperty::PaddingBottom(lerp(*a, *b, t))
        }
        (AnimatableProperty::PaddingLeft(a), AnimatableProperty::PaddingLeft(b)) => {
            AnimatableProperty::PaddingLeft(lerp(*a, *b, t))
        }
        (AnimatableProperty::MarginTop(a), AnimatableProperty::MarginTop(b)) => {
            AnimatableProperty::MarginTop(lerp(*a, *b, t))
        }
        (AnimatableProperty::MarginRight(a), AnimatableProperty::MarginRight(b)) => {
            AnimatableProperty::MarginRight(lerp(*a, *b, t))
        }
        (AnimatableProperty::MarginBottom(a), AnimatableProperty::MarginBottom(b)) => {
            AnimatableProperty::MarginBottom(lerp(*a, *b, t))
        }
        (AnimatableProperty::MarginLeft(a), AnimatableProperty::MarginLeft(b)) => {
            AnimatableProperty::MarginLeft(lerp(*a, *b, t))
        }
        // Mismatched types — just return the 'to' value.
        _ => to.clone(),
    }
}

/// Apply an interpolated property to a CXRD node's style.
pub fn apply_animated_property(
    style: &mut crate::cxrd::style::ComputedStyle,
    prop: &AnimatableProperty,
) {
    match prop {
        AnimatableProperty::Opacity(v) => style.opacity = *v,
        AnimatableProperty::BackgroundColor(c) => {
            style.background = crate::cxrd::style::Background::Solid(*c);
        }
        AnimatableProperty::Color(c) => style.color = *c,
        AnimatableProperty::BorderColor(c) => style.border_color = *c,
        AnimatableProperty::BorderRadius(v) => {
            style.border_radius = crate::cxrd::value::CornerRadii::uniform(*v);
        }
        AnimatableProperty::Width(v) => {
            style.width = crate::cxrd::value::Dimension::Px(*v);
        }
        AnimatableProperty::Height(v) => {
            style.height = crate::cxrd::value::Dimension::Px(*v);
        }
        AnimatableProperty::FontSize(v) => style.font_size = *v,
        AnimatableProperty::Gap(v) => style.gap = *v,
        // Transform properties don't directly map to style yet.
        // They'll be applied via a transform matrix on the instance.
        _ => {}
    }
}

#[inline]
fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}
