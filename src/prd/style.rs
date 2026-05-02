// prism-runtime/src/prd/style.rs
//
// Computed style properties for PRD nodes.
// These are fully resolved — no cascading, no inheritance at render time.
// The compiler resolves all CSS into computed styles during compilation.

use serde::{Serialize, Deserialize};
use crate::prd::value::{Color, Dimension, EdgeInsets, CornerRadii};

/// Pseudo-class category for runtime behavior dispatch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PseudoClassCategory {
    /// Requires mouse/keyboard/focus tracking (hover, active, focus, etc.).
    Interactive,
    /// Requires DOM tree position analysis (first-child, nth-child, etc.).
    Structural,
    /// Requires input/form element state tracking (checked, disabled, etc.).
    FormState,
    /// Requires link state tracking (link, visited, etc.).
    LinkState,
    /// Requires media/document state (fullscreen, playing, etc.).
    MediaState,
    /// Functional selectors (:is, :not, :has, :where).
    Functional,
    /// Element state / host / directionality / target.
    ElementState,
    /// Unrecognized pseudo-class.
    Unknown,
}

/// Display mode for a node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Display {
    Flex,
    InlineFlex,
    Grid,
    InlineGrid,
    Block,
    Inline,
    InlineBlock,
    None,
}

impl Default for Display {
    fn default() -> Self {
        Display::Block
    }
}

/// Flex direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FlexDirection {
    Row,
    RowReverse,
    Column,
    ColumnReverse,
}

impl Default for FlexDirection {
    fn default() -> Self {
        FlexDirection::Row
    }
}

/// Flex wrap mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FlexWrap {
    NoWrap,
    Wrap,
    WrapReverse,
}

impl Default for FlexWrap {
    fn default() -> Self {
        FlexWrap::NoWrap
    }
}

/// Justify-content values (main-axis alignment).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum JustifyContent {
    FlexStart,
    FlexEnd,
    Center,
    SpaceBetween,
    SpaceAround,
    SpaceEvenly,
}

impl Default for JustifyContent {
    fn default() -> Self {
        JustifyContent::FlexStart
    }
}

/// Align-items values (cross-axis alignment).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AlignItems {
    FlexStart,
    FlexEnd,
    Center,
    Stretch,
    Baseline,
}

impl Default for AlignItems {
    fn default() -> Self {
        AlignItems::Stretch
    }
}

/// Align-self (per-child cross-axis override).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AlignSelf {
    Auto,
    FlexStart,
    FlexEnd,
    Center,
    Stretch,
}

impl Default for AlignSelf {
    fn default() -> Self {
        AlignSelf::Auto
    }
}

/// Positioning mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Position {
    Static,
    Relative,
    Absolute,
    Fixed,
    Sticky,
}

impl Default for Position {
    fn default() -> Self {
        Position::Static
    }
}

/// Overflow behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Overflow {
    Visible,
    Hidden,
    Scroll,
}

impl Default for Overflow {
    fn default() -> Self {
        Overflow::Visible
    }
}

/// Text alignment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TextAlign {
    Left,
    Center,
    Right,
}

/// Text transform.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TextTransform {
    None,
    Uppercase,
    Lowercase,
    Capitalize,
}

impl Default for TextTransform {
    fn default() -> Self {
        TextTransform::None
    }
}

/// White-space handling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WhiteSpace {
    Normal,
    NoWrap,
    Pre,
    PreWrap,
    PreLine,
}

impl Default for WhiteSpace {
    fn default() -> Self {
        WhiteSpace::Normal
    }
}

/// A CSS grid track size.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum GridTrackSize {
    Auto,
    Px(f32),
    Percent(f32),
    Fr(f32),
    MinContent,
    MaxContent,
}

impl Default for TextAlign {
    fn default() -> Self {
        TextAlign::Left
    }
}

/// Font weight (100–900).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FontWeight(pub u16);

impl Default for FontWeight {
    fn default() -> Self {
        FontWeight(400)
    }
}

/// Font style (normal, italic, oblique).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FontStyle {
    Normal,
    Italic,
    Oblique,
}

impl Default for FontStyle {
    fn default() -> Self { FontStyle::Normal }
}

/// Border line style.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BorderStyle {
    None,
    Solid,
    Dashed,
    Dotted,
    Double,
    Groove,
    Ridge,
    Inset,
    Outset,
    Hidden,
}

impl Default for BorderStyle {
    fn default() -> Self { BorderStyle::Solid }
}

/// Visibility mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Visibility {
    Visible,
    Hidden,
    Collapse,
}

impl Default for Visibility {
    fn default() -> Self { Visibility::Visible }
}

/// Pointer events mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PointerEvents {
    Auto,
    None,
}

impl Default for PointerEvents {
    fn default() -> Self { PointerEvents::Auto }
}

/// Text overflow mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TextOverflow {
    Clip,
    Ellipsis,
}

impl Default for TextOverflow {
    fn default() -> Self { TextOverflow::Clip }
}

/// Text decoration line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TextDecoration {
    None,
    Underline,
    LineThrough,
    Overline,
}

impl Default for TextDecoration {
    fn default() -> Self { TextDecoration::None }
}

/// Cursor style hint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CursorStyle {
    Auto,
    Default,
    Pointer,
    Text,
    Move,
    NotAllowed,
    Grab,
    Grabbing,
    CrossHair,
    ColResize,
    RowResize,
    NsResize,
    EwResize,
}

impl Default for CursorStyle {
    fn default() -> Self { CursorStyle::Auto }
}

/// Object-fit for images.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ObjectFit {
    Fill,
    Contain,
    Cover,
    ScaleDown,
    None,
}

impl Default for ObjectFit {
    fn default() -> Self { ObjectFit::Fill }
}

/// Align-content for flex/grid containers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AlignContent {
    FlexStart,
    FlexEnd,
    Center,
    Stretch,
    SpaceBetween,
    SpaceAround,
    SpaceEvenly,
}

impl Default for AlignContent {
    fn default() -> Self { AlignContent::Stretch }
}

/// Background size mode.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum BackgroundSize {
    Auto,
    Cover,
    Contain,
    Explicit(f32, f32),
}

impl Default for BackgroundSize {
    fn default() -> Self { BackgroundSize::Auto }
}

/// Background position axis value.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum BackgroundPosition {
    Px(f32),
    Percent(f32),
    Center,
}

impl Default for BackgroundPosition {
    fn default() -> Self { BackgroundPosition::Percent(0.0) }
}

/// Background repeat mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BackgroundRepeat {
    Repeat,
    NoRepeat,
    RepeatX,
    RepeatY,
}

impl Default for BackgroundRepeat {
    fn default() -> Self { BackgroundRepeat::Repeat }
}

/// Box-sizing mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BoxSizing {
    ContentBox,
    BorderBox,
}

impl Default for BoxSizing {
    fn default() -> Self { BoxSizing::ContentBox }
}

/// Background specification.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Background {
    None,
    Solid(Color),
    LinearGradient {
        angle_deg: f32,
        stops: Vec<GradientStop>,
    },
    RadialGradient {
        stops: Vec<GradientStop>,
    },
    Image {
        /// Index into the PRD asset table.
        asset_index: u32,
    },
}

impl Default for Background {
    fn default() -> Self {
        Background::None
    }
}

/// A gradient color stop.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct GradientStop {
    pub color: Color,
    pub position: f32, // 0.0–1.0
}

/// Box shadow.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct BoxShadow {
    pub offset_x: f32,
    pub offset_y: f32,
    pub blur_radius: f32,
    pub spread_radius: f32,
    pub color: Color,
    pub inset: bool,
}

/// CSS transition definition (compiled from CSS `transition` shorthand).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TransitionDef {
    pub property: String,
    pub duration_ms: f32,
    pub delay_ms: f32,
    pub easing: EasingFunction,
}

/// Easing function for transitions and animations.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum EasingFunction {
    Linear,
    Ease,
    EaseIn,
    EaseOut,
    EaseInOut,
    CubicBezier(f32, f32, f32, f32),
}

impl Default for EasingFunction {
    fn default() -> Self {
        EasingFunction::Ease
    }
}

/// User-select mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum UserSelect {
    Auto,
    None,
    Text,
    All,
}

impl Default for UserSelect {
    fn default() -> Self { UserSelect::Auto }
}

/// Float mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Float {
    None,
    Left,
    Right,
    InlineStart,
    InlineEnd,
}

impl Default for Float {
    fn default() -> Self { Float::None }
}

/// Clear mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Clear {
    None,
    Left,
    Right,
    Both,
    InlineStart,
    InlineEnd,
}

impl Default for Clear {
    fn default() -> Self { Clear::None }
}

/// Isolation mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Isolation {
    Auto,
    Isolate,
}

impl Default for Isolation {
    fn default() -> Self { Isolation::Auto }
}

/// Blend mode (for mix-blend-mode / background-blend-mode).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BlendMode {
    Normal,
    Multiply,
    Screen,
    Overlay,
    Darken,
    Lighten,
    ColorDodge,
    ColorBurn,
    HardLight,
    SoftLight,
    Difference,
    Exclusion,
    Hue,
    Saturation,
    Color,
    Luminosity,
}

impl Default for BlendMode {
    fn default() -> Self { BlendMode::Normal }
}

/// Text decoration style.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TextDecorationStyle {
    Solid,
    Double,
    Dotted,
    Dashed,
    Wavy,
}

impl Default for TextDecorationStyle {
    fn default() -> Self { TextDecorationStyle::Solid }
}

/// Grid auto-flow mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GridAutoFlow {
    Row,
    Column,
    Dense,
    RowDense,
    ColumnDense,
}

impl Default for GridAutoFlow {
    fn default() -> Self { GridAutoFlow::Row }
}

/// Backface visibility.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BackfaceVisibility {
    Visible,
    Hidden,
}

impl Default for BackfaceVisibility {
    fn default() -> Self { BackfaceVisibility::Visible }
}

/// Background attachment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BackgroundAttachment {
    Scroll,
    Fixed,
    Local,
}

impl Default for BackgroundAttachment {
    fn default() -> Self { BackgroundAttachment::Scroll }
}

/// Background clip / origin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BackgroundBox {
    BorderBox,
    PaddingBox,
    ContentBox,
}

impl Default for BackgroundBox {
    fn default() -> Self { BackgroundBox::BorderBox }
}

/// Word-break mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WordBreak {
    Normal,
    BreakAll,
    KeepAll,
    BreakWord,
}

impl Default for WordBreak {
    fn default() -> Self { WordBreak::Normal }
}

/// Overflow-wrap mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OverflowWrap {
    Normal,
    BreakWord,
    Anywhere,
}

impl Default for OverflowWrap {
    fn default() -> Self { OverflowWrap::Normal }
}

/// CSS transform function.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum CssTransform {
    Translate(f32, f32),
    Scale(f32, f32),
    Rotate(f32),
    SkewX(f32),
    SkewY(f32),
}

/// CSS filter function.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum CssFilter {
    Blur(f32),
    Brightness(f32),
    Contrast(f32),
    Grayscale(f32),
    HueRotate(f32),
    Invert(f32),
    Opacity(f32),
    Saturate(f32),
    Sepia(f32),
}

/// Animation iteration count.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum AnimationIterationCount {
    Number(f32),
    Infinite,
}

impl Default for AnimationIterationCount {
    fn default() -> Self { AnimationIterationCount::Number(1.0) }
}

/// Animation direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AnimationDirection {
    Normal,
    Reverse,
    Alternate,
    AlternateReverse,
}

impl Default for AnimationDirection {
    fn default() -> Self { AnimationDirection::Normal }
}

/// Animation fill mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AnimationFillMode {
    None,
    Forwards,
    Backwards,
    Both,
}

impl Default for AnimationFillMode {
    fn default() -> Self { AnimationFillMode::None }
}

/// Animation play state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AnimationPlayState {
    Running,
    Paused,
}

impl Default for AnimationPlayState {
    fn default() -> Self { AnimationPlayState::Running }
}

/// CSS animation definition.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnimationDef {
    pub name: String,
    pub duration_ms: f32,
    pub delay_ms: f32,
    pub easing: EasingFunction,
    pub iteration_count: AnimationIterationCount,
    pub direction: AnimationDirection,
    pub fill_mode: AnimationFillMode,
    pub play_state: AnimationPlayState,
}

/// The fully-computed style for a PRD node.
/// Every field is resolved — no inheritance lookups, no cascade.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ComputedStyle {
    // --- Layout ---
    pub display: Display,
    pub position: Position,
    pub overflow: Overflow,
    pub overflow_x: Overflow,
    pub overflow_y: Overflow,
    pub float: Float,
    pub clear: Clear,

    pub width: Dimension,
    pub height: Dimension,
    pub min_width: Dimension,
    pub min_height: Dimension,
    pub max_width: Dimension,
    pub max_height: Dimension,

    pub margin: EdgeInsetsD,
    pub padding: EdgeInsetsD,

    // --- Flex ---
    pub flex_direction: FlexDirection,
    pub flex_wrap: FlexWrap,
    pub justify_content: JustifyContent,
    pub align_items: AlignItems,
    pub align_self: AlignSelf,
    pub flex_grow: f32,
    pub flex_shrink: f32,
    pub flex_basis: Dimension,
    pub gap: f32,

    // --- Position offsets (for absolute / fixed) ---
    pub top: Dimension,
    pub right: Dimension,
    pub bottom: Dimension,
    pub left: Dimension,

    // --- Visual ---
    pub background: Background,
    pub border_color: Color,
    pub border_width: EdgeInsets,
    pub border_radius: CornerRadii,
    pub border_style: BorderStyle,
    pub box_shadow: Vec<BoxShadow>,
    pub backdrop_blur: f32,
    pub transform_scale: f32,
    pub opacity: f32,

    // --- Grid ---
    pub grid_template_columns: Vec<GridTrackSize>,
    pub grid_template_rows: Vec<GridTrackSize>,
    pub grid_template_areas: Vec<String>,
    pub grid_auto_flow: GridAutoFlow,
    pub grid_auto_columns: Vec<GridTrackSize>,
    pub grid_auto_rows: Vec<GridTrackSize>,
    pub grid_column_start: i32,
    pub grid_column_end: i32,
    pub grid_row_start: i32,
    pub grid_row_end: i32,

    // --- Typography ---
    pub color: Color,
    pub font_family: String,
    pub font_size: f32,      // px, resolved
    pub font_weight: FontWeight,
    pub font_style: FontStyle,
    pub line_height: f32,    // multiplier
    pub text_align: TextAlign,
    pub letter_spacing: f32,
    pub word_spacing: f32,
    pub text_transform: TextTransform,
    pub white_space: WhiteSpace,
    pub word_break: WordBreak,
    pub overflow_wrap: OverflowWrap,

    // --- Transitions ---
    pub transitions: Vec<TransitionDef>,

    // --- Animations ---
    pub animations: Vec<AnimationDef>,

    // --- Z-index (for stacking context) ---
    pub z_index: i32,

    // --- Extended properties ---
    /// Per-side border colors (overrides uniform border_color when set).
    pub border_top_color: Option<Color>,
    pub border_right_color: Option<Color>,
    pub border_bottom_color: Option<Color>,
    pub border_left_color: Option<Color>,

    /// Per-side border widths (override uniform border_width when set).
    pub border_top_width: Option<f32>,
    pub border_right_width: Option<f32>,
    pub border_bottom_width: Option<f32>,
    pub border_left_width: Option<f32>,

    /// Per-side border styles.
    pub border_top_style: Option<BorderStyle>,
    pub border_right_style: Option<BorderStyle>,
    pub border_bottom_style: Option<BorderStyle>,
    pub border_left_style: Option<BorderStyle>,

    /// Visibility (hidden elements take up space but aren't painted).
    pub visibility: Visibility,

    /// Pointer-events (none = click-through).
    pub pointer_events: PointerEvents,

    /// User-select mode.
    pub user_select: UserSelect,

    /// Text overflow (ellipsis truncation).
    pub text_overflow: TextOverflow,

    /// Text decoration (underline, line-through, etc.).
    pub text_decoration: TextDecoration,
    /// Text decoration color.
    pub text_decoration_color: Option<Color>,
    /// Text decoration style.
    pub text_decoration_style: TextDecorationStyle,
    /// Text decoration thickness.
    pub text_decoration_thickness: Option<f32>,

    /// Cursor style hint.
    pub cursor: CursorStyle,

    /// Object-fit for images.
    pub object_fit: ObjectFit,
    /// Object-position.
    pub object_position: (BackgroundPosition, BackgroundPosition),

    /// Align-content (for flex containers with wrapped lines).
    pub align_content: AlignContent,

    /// Order for flex / grid items.
    pub order: i32,

    /// Row gap for grid / flex containers.
    pub row_gap: f32,
    /// Column gap for grid / flex containers.
    pub column_gap: f32,

    /// Background image URL (external reference, not asset-bundled).
    pub background_image: Option<String>,
    /// Background size mode.
    pub background_size: BackgroundSize,
    /// Background position.
    pub background_position: (BackgroundPosition, BackgroundPosition),
    /// Background repeat.
    pub background_repeat: BackgroundRepeat,
    /// Background attachment.
    pub background_attachment: BackgroundAttachment,
    /// Background clip.
    pub background_clip: BackgroundBox,
    /// Background origin.
    pub background_origin: BackgroundBox,
    /// Background blend mode.
    pub background_blend_mode: BlendMode,

    /// Outline color.
    pub outline_color: Option<Color>,
    /// Outline width.
    pub outline_width: f32,
    /// Outline offset.
    pub outline_offset: f32,
    /// Outline style.
    pub outline_style: BorderStyle,

    /// Aspect ratio (e.g., 16/9 → 1.777).
    pub aspect_ratio: Option<f32>,

    /// Box-sizing mode.
    pub box_sizing: BoxSizing,

    // --- Transform ---
    pub transforms: Vec<CssTransform>,
    pub transform_origin: (BackgroundPosition, BackgroundPosition),
    pub backface_visibility: BackfaceVisibility,
    pub perspective: Option<f32>,

    // --- Filter / Blend ---
    pub filters: Vec<CssFilter>,
    pub isolation: Isolation,
    pub mix_blend_mode: BlendMode,

    // --- SVG ---
    pub svg_fill: Option<Color>,
    pub svg_stroke: Option<Color>,
    pub svg_stroke_width: Option<f32>,

    // --- Color ---
    pub accent_color: Option<Color>,
    pub caret_color: Option<Color>,
    pub color_scheme: Option<String>,

    // --- Scroll ---
    pub scroll_behavior: Option<String>,

    // --- Containment ---
    pub contain: Option<String>,
    pub content_visibility: Option<String>,
    pub will_change: Option<String>,

    // --- Font (extended) ---
    pub font_variant: Option<String>,
    pub font_stretch: Option<String>,
}

/// Edge insets in dimension form (before resolution to px).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct EdgeInsetsD {
    pub top: Dimension,
    pub right: Dimension,
    pub bottom: Dimension,
    pub left: Dimension,
}

impl Default for EdgeInsetsD {
    fn default() -> Self {
        Self {
            top: Dimension::Px(0.0),
            right: Dimension::Px(0.0),
            bottom: Dimension::Px(0.0),
            left: Dimension::Px(0.0),
        }
    }
}

impl Default for ComputedStyle {
    fn default() -> Self {
        Self {
            display: Display::default(),
            position: Position::default(),
            overflow: Overflow::default(),
            overflow_x: Overflow::default(),
            overflow_y: Overflow::default(),
            float: Float::default(),
            clear: Clear::default(),
            width: Dimension::Auto,
            height: Dimension::Auto,
            min_width: Dimension::Px(0.0),
            min_height: Dimension::Px(0.0),
            max_width: Dimension::Auto,
            max_height: Dimension::Auto,
            margin: EdgeInsetsD::default(),
            padding: EdgeInsetsD::default(),
            flex_direction: FlexDirection::default(),
            flex_wrap: FlexWrap::default(),
            justify_content: JustifyContent::default(),
            align_items: AlignItems::default(),
            align_self: AlignSelf::default(),
            flex_grow: 0.0,
            flex_shrink: 1.0,
            flex_basis: Dimension::Auto,
            gap: 0.0,
            grid_template_columns: Vec::new(),
            grid_template_rows: Vec::new(),
            grid_template_areas: Vec::new(),
            grid_auto_flow: GridAutoFlow::default(),
            grid_auto_columns: Vec::new(),
            grid_auto_rows: Vec::new(),
            grid_column_start: 0,
            grid_column_end: 0,
            grid_row_start: 0,
            grid_row_end: 0,
            top: Dimension::Auto,
            right: Dimension::Auto,
            bottom: Dimension::Auto,
            left: Dimension::Auto,
            background: Background::default(),
            border_color: Color::TRANSPARENT,
            border_width: EdgeInsets::default(),
            border_radius: CornerRadii::default(),
            border_style: BorderStyle::default(),
            box_shadow: Vec::new(),
            backdrop_blur: 0.0,
            transform_scale: 1.0,
            opacity: 1.0,
            color: Color::WHITE,
            font_family: String::new(),
            font_size: 16.0,
            font_weight: FontWeight::default(),
            font_style: FontStyle::default(),
            line_height: 1.2,
            text_align: TextAlign::default(),
            letter_spacing: 0.0,
            word_spacing: 0.0,
            text_transform: TextTransform::default(),
            white_space: WhiteSpace::default(),
            word_break: WordBreak::default(),
            overflow_wrap: OverflowWrap::default(),
            transitions: Vec::new(),
            animations: Vec::new(),
            z_index: 0,
            border_top_color: None,
            border_right_color: None,
            border_bottom_color: None,
            border_left_color: None,
            border_top_width: None,
            border_right_width: None,
            border_bottom_width: None,
            border_left_width: None,
            border_top_style: None,
            border_right_style: None,
            border_bottom_style: None,
            border_left_style: None,
            visibility: Visibility::default(),
            pointer_events: PointerEvents::default(),
            user_select: UserSelect::default(),
            text_overflow: TextOverflow::default(),
            text_decoration: TextDecoration::default(),
            text_decoration_color: None,
            text_decoration_style: TextDecorationStyle::default(),
            text_decoration_thickness: None,
            cursor: CursorStyle::default(),
            object_fit: ObjectFit::default(),
            object_position: (BackgroundPosition::default(), BackgroundPosition::default()),
            align_content: AlignContent::default(),
            order: 0,
            row_gap: 0.0,
            column_gap: 0.0,
            background_image: None,
            background_size: BackgroundSize::default(),
            background_position: (BackgroundPosition::default(), BackgroundPosition::default()),
            background_repeat: BackgroundRepeat::default(),
            background_attachment: BackgroundAttachment::default(),
            background_clip: BackgroundBox::default(),
            background_origin: BackgroundBox::default(),
            background_blend_mode: BlendMode::default(),
            outline_color: None,
            outline_width: 0.0,
            outline_offset: 0.0,
            outline_style: BorderStyle::default(),
            aspect_ratio: None,
            box_sizing: BoxSizing::default(),
            transforms: Vec::new(),
            transform_origin: (BackgroundPosition::default(), BackgroundPosition::default()),
            backface_visibility: BackfaceVisibility::default(),
            perspective: None,
            filters: Vec::new(),
            isolation: Isolation::default(),
            mix_blend_mode: BlendMode::default(),
            svg_fill: None,
            svg_stroke: None,
            svg_stroke_width: None,
            accent_color: None,
            caret_color: None,
            color_scheme: None,
            scroll_behavior: None,
            contain: None,
            content_visibility: None,
            will_change: None,
            font_variant: None,
            font_stretch: None,
        }
    }
}

