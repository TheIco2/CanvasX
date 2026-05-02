// prism-runtime/src/prd/animation.rs
//
// Animation definitions within a PRD document.
// These are compiled from CSS @keyframes and transition definitions.

use serde::{Serialize, Deserialize};
use crate::prd::style::EasingFunction;
use crate::prd::value::Color;

/// A compiled animation track.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnimationDef {
    /// Name of this animation (from @keyframes name).
    pub name: String,

    /// Duration in milliseconds.
    pub duration_ms: f32,

    /// Delay before starting, in milliseconds.
    pub delay_ms: f32,

    /// Number of iterations (f32::INFINITY for infinite).
    pub iteration_count: f32,

    /// Direction.
    pub direction: AnimationDirection,

    /// Fill mode.
    pub fill_mode: AnimationFillMode,

    /// The keyframes, sorted by time offset (0.0–1.0).
    pub keyframes: Vec<Keyframe>,
}

/// Animation playback direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AnimationDirection {
    Normal,
    Reverse,
    Alternate,
    AlternateReverse,
}

impl Default for AnimationDirection {
    fn default() -> Self {
        AnimationDirection::Normal
    }
}

/// Fill mode — what happens before/after the animation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AnimationFillMode {
    None,
    Forwards,
    Backwards,
    Both,
}

impl Default for AnimationFillMode {
    fn default() -> Self {
        AnimationFillMode::None
    }
}

/// A single keyframe in an animation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Keyframe {
    /// Normalized time offset (0.0 = start, 1.0 = end).
    pub offset: f32,

    /// Easing to use from this keyframe to the next.
    pub easing: EasingFunction,

    /// Property values at this keyframe.
    pub properties: Vec<AnimatableProperty>,
}

/// Properties that can be animated.
/// These are the limited set the GPU runtime can interpolate per frame.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum AnimatableProperty {
    Opacity(f32),
    TranslateX(f32),
    TranslateY(f32),
    ScaleX(f32),
    ScaleY(f32),
    Rotate(f32), // degrees
    BackgroundColor(Color),
    Color(Color),
    BorderColor(Color),
    BorderRadius(f32),
    Width(f32),
    Height(f32),
    FontSize(f32),
    Gap(f32),
    PaddingTop(f32),
    PaddingRight(f32),
    PaddingBottom(f32),
    PaddingLeft(f32),
    MarginTop(f32),
    MarginRight(f32),
    MarginBottom(f32),
    MarginLeft(f32),
}

impl AnimatableProperty {
    /// Get the property name (for matching with CSS transition-property).
    pub fn name(&self) -> &'static str {
        match self {
            AnimatableProperty::Opacity(_) => "opacity",
            AnimatableProperty::TranslateX(_) => "transform.translateX",
            AnimatableProperty::TranslateY(_) => "transform.translateY",
            AnimatableProperty::ScaleX(_) => "transform.scaleX",
            AnimatableProperty::ScaleY(_) => "transform.scaleY",
            AnimatableProperty::Rotate(_) => "transform.rotate",
            AnimatableProperty::BackgroundColor(_) => "background-color",
            AnimatableProperty::Color(_) => "color",
            AnimatableProperty::BorderColor(_) => "border-color",
            AnimatableProperty::BorderRadius(_) => "border-radius",
            AnimatableProperty::Width(_) => "width",
            AnimatableProperty::Height(_) => "height",
            AnimatableProperty::FontSize(_) => "font-size",
            AnimatableProperty::Gap(_) => "gap",
            AnimatableProperty::PaddingTop(_) => "padding-top",
            AnimatableProperty::PaddingRight(_) => "padding-right",
            AnimatableProperty::PaddingBottom(_) => "padding-bottom",
            AnimatableProperty::PaddingLeft(_) => "padding-left",
            AnimatableProperty::MarginTop(_) => "margin-top",
            AnimatableProperty::MarginRight(_) => "margin-right",
            AnimatableProperty::MarginBottom(_) => "margin-bottom",
            AnimatableProperty::MarginLeft(_) => "margin-left",
        }
    }
}

