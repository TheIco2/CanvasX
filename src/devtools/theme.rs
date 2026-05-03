// prism-runtime/src/devtools/theme.rs
//
// Chrome-DevTools-inspired theme tokens for the in-window DevTools.
// Centralised so the UI has a single source of truth for colors, spacing,
// typography, and stroke widths.

use crate::prd::value::Color;

// ---------------------------------------------------------------------------
// Surface / chrome
// ---------------------------------------------------------------------------
pub const BG_PANEL:       Color = Color { r: 0.125, g: 0.129, b: 0.137, a: 0.98 }; // #202124
pub const BG_TOOLBAR:     Color = Color { r: 0.161, g: 0.165, b: 0.176, a: 1.0  }; // #292a2d
pub const BG_TAB_BAR:     Color = Color { r: 0.094, g: 0.098, b: 0.106, a: 1.0  }; // #18191b
pub const BG_TAB_ACTIVE:  Color = Color { r: 0.125, g: 0.129, b: 0.137, a: 1.0  };
pub const BG_TAB_HOVER:   Color = Color { r: 0.180, g: 0.188, b: 0.200, a: 1.0  };
pub const BG_ROW_HOVER:   Color = Color { r: 1.0,   g: 1.0,   b: 1.0,   a: 0.04 };
pub const BG_ROW_SELECT:  Color = Color { r: 0.137, g: 0.235, b: 0.388, a: 1.0  }; // #233c63
pub const BG_INPUT:       Color = Color { r: 0.078, g: 0.082, b: 0.090, a: 1.0  };
pub const BG_BADGE:       Color = Color { r: 0.094, g: 0.098, b: 0.106, a: 0.85 };

pub const LINE:           Color = Color { r: 0.235, g: 0.243, b: 0.255, a: 1.0  }; // #3c4043
pub const LINE_SOFT:      Color = Color { r: 0.180, g: 0.188, b: 0.200, a: 1.0  };

// ---------------------------------------------------------------------------
// Foreground / text
// ---------------------------------------------------------------------------
pub const TEXT_PRIMARY:   Color = Color { r: 0.910, g: 0.910, b: 0.918, a: 1.0  }; // #e8eaed
pub const TEXT_SECONDARY: Color = Color { r: 0.616, g: 0.631, b: 0.659, a: 1.0  }; // #9aa0a6
pub const TEXT_MUTED:     Color = Color { r: 0.435, g: 0.443, b: 0.471, a: 1.0  }; // #6f7278
pub const TEXT_DISABLED:  Color = Color { r: 0.353, g: 0.361, b: 0.388, a: 1.0  };
pub const TEXT_INVERSE:   Color = Color { r: 0.094, g: 0.098, b: 0.106, a: 1.0  };

// ---------------------------------------------------------------------------
// Accents (Chrome DevTools palette)
// ---------------------------------------------------------------------------
pub const ACCENT:         Color = Color { r: 0.541, g: 0.706, b: 0.973, a: 1.0  }; // #8ab4f8 (links / active tab indicator)
pub const ACCENT_HOVER:   Color = Color { r: 0.667, g: 0.792, b: 1.000, a: 1.0  };
pub const ACCENT_DIM:     Color = Color { r: 0.541, g: 0.706, b: 0.973, a: 0.35 };

pub const SEVERE_ERROR:   Color = Color { r: 0.961, g: 0.420, b: 0.392, a: 1.0  }; // #f56b64
pub const SEVERE_WARN:    Color = Color { r: 0.984, g: 0.737, b: 0.020, a: 1.0  }; // #fbbc04
pub const SEVERE_INFO:    Color = Color { r: 0.541, g: 0.706, b: 0.973, a: 1.0  };
pub const SEVERE_DEBUG:   Color = Color { r: 0.616, g: 0.631, b: 0.659, a: 1.0  };

// Box-model overlay tints (Chrome canonical)
pub const BOX_MARGIN:     Color = Color { r: 0.969, g: 0.706, b: 0.498, a: 0.35 }; // #f7b47f
pub const BOX_BORDER:     Color = Color { r: 0.984, g: 0.886, b: 0.255, a: 0.35 }; // #fbe241
pub const BOX_PADDING:    Color = Color { r: 0.580, g: 0.851, b: 0.561, a: 0.35 }; // #94d98f
pub const BOX_CONTENT:    Color = Color { r: 0.451, g: 0.671, b: 0.929, a: 0.35 }; // #73abec

// ---------------------------------------------------------------------------
// Syntax colors (Elements tree, Computed styles)
// ---------------------------------------------------------------------------
pub const SYN_TAG:        Color = Color { r: 0.561, g: 0.812, b: 0.961, a: 1.0  }; // #8fcff5
pub const SYN_ATTR_NAME:  Color = Color { r: 0.808, g: 0.788, b: 0.557, a: 1.0  }; // #cec98e
pub const SYN_ATTR_VAL:   Color = Color { r: 0.973, g: 0.706, b: 0.557, a: 1.0  }; // #f8b48e
pub const SYN_TEXT:       Color = Color { r: 0.741, g: 0.776, b: 0.812, a: 1.0  };
pub const SYN_PUNCT:      Color = Color { r: 0.616, g: 0.631, b: 0.659, a: 1.0  };
pub const SYN_PROP_NAME:  Color = Color { r: 0.553, g: 0.776, b: 0.965, a: 1.0  };
pub const SYN_PROP_VAL:   Color = Color { r: 0.910, g: 0.910, b: 0.918, a: 1.0  };
pub const SYN_NUMBER:     Color = Color { r: 0.957, g: 0.812, b: 0.451, a: 1.0  };
pub const SYN_STRING:     Color = Color { r: 0.973, g: 0.741, b: 0.557, a: 1.0  };
pub const SYN_KEYWORD:    Color = Color { r: 0.749, g: 0.616, b: 0.918, a: 1.0  };

// ---------------------------------------------------------------------------
// Typography
// ---------------------------------------------------------------------------
pub const FONT_TINY:   f32 = 10.0;
pub const FONT_SMALL:  f32 = 11.0;
pub const FONT_BODY:   f32 = 12.0;
pub const FONT_HEADER: f32 = 13.0;

pub const LINE_HEIGHT: f32 = 1.4;

// ---------------------------------------------------------------------------
// Spacing scale (consistent rhythm)
// ---------------------------------------------------------------------------
pub const SP_1: f32 = 2.0;
pub const SP_2: f32 = 4.0;
pub const SP_3: f32 = 6.0;
pub const SP_4: f32 = 8.0;
pub const SP_5: f32 = 12.0;
pub const SP_6: f32 = 16.0;
pub const SP_7: f32 = 24.0;

// ---------------------------------------------------------------------------
// Chrome dimensions
// ---------------------------------------------------------------------------
pub const PANEL_HEIGHT_DEFAULT: f32 = 360.0;
pub const PANEL_WIDTH_DEFAULT:  f32 = 480.0;
pub const PANEL_MIN_HEIGHT:     f32 = 120.0;
pub const PANEL_MIN_WIDTH:      f32 = 280.0;

pub const TOOLBAR_HEIGHT: f32 = 26.0;
pub const TAB_BAR_HEIGHT: f32 = 28.0;
pub const TAB_HEIGHT:     f32 = 28.0;
pub const TAB_PADDING_X:  f32 = 14.0;
pub const TAB_INDICATOR:  f32 = 2.0;

pub const ROW_HEIGHT:     f32 = 18.0;
pub const ROW_INDENT:     f32 = 14.0;

pub const SCROLLBAR_W:    f32 = 8.0;
pub const SCROLLBAR_MIN:  f32 = 24.0;

pub const SPLITTER:       f32 = 4.0;

pub const RESIZE_HANDLE:  f32 = 4.0;

pub const BADGE_W: f32 = 56.0;
pub const BADGE_H: f32 = 20.0;
pub const BADGE_MARGIN: f32 = 8.0;

pub const SIDEBAR_W: f32 = 320.0;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Mix `a` and `b` by `t` in 0..=1.
pub fn mix(a: Color, b: Color, t: f32) -> Color {
    let t = t.clamp(0.0, 1.0);
    Color {
        r: a.r + (b.r - a.r) * t,
        g: a.g + (b.g - a.g) * t,
        b: a.b + (b.b - a.b) * t,
        a: a.a + (b.a - a.a) * t,
    }
}

/// Return a new color with a different alpha.
pub fn alpha(c: Color, a: f32) -> Color {
    Color { r: c.r, g: c.g, b: c.b, a }
}
