// Copyright 2026 Matt Franklin
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! What a radio's console looks like, said by the radio's own server.
//!
//! The other half of a layout. A layout says where the panels go; this
//! says what they look like — and for the same reason, it is the radio's
//! to decide rather than a renderer's.
//!
//! # Why a radio has a look at all
//!
//! An operator sitting in front of a TS-570D is looking at an amber LCD on
//! a charcoal panel. An FT-991A is a colour TFT, blue and white. Those are
//! facts about hardware somebody owns, and a console that matches its rig
//! is a console whose readout an operator can find without translating
//! from one visual language to another.
//!
//! This is **not** decoration for its own sake, and it is not a licence to
//! restyle the components. The structure, the type scale and the spacing
//! are the design system's and stay shared. What a radio picks is the
//! palette its own front panel uses.
//!
//! # Colours, not styles
//!
//! A radio names colours by role — ink, accent, the trace on a spectrum —
//! and a renderer decides what to do with them. A radio that could send
//! stylesheets would be a radio that could break a console, and the point
//! of a shared design system is that it cannot.

use serde::{Deserialize, Serialize};

/// A colour, as a radio names one.
///
/// Plain sRGB bytes: this crate has no renderer in it and must not depend
/// on one's colour type. `egui::Color32` and ratatui's `Color` are both
/// one conversion away.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb {
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    /// From `0xRRGGBB`, which is how a palette is usually written down.
    pub const fn hex(v: u32) -> Self {
        Self {
            r: ((v >> 16) & 0xff) as u8,
            g: ((v >> 8) & 0xff) as u8,
            b: (v & 0xff) as u8,
        }
    }

    pub const fn to_tuple(self) -> (u8, u8, u8) {
        (self.r, self.g, self.b)
    }

    /// Mix towards `other`. `amount` is clamped to 0.0..=1.0.
    ///
    /// For the shades a renderer needs and a radio should not have to
    /// enumerate — a pressed button, a dimmed row — derived from the
    /// palette rather than added to it, so a radio names six colours
    /// instead of twenty.
    pub fn mix(self, other: Rgb, amount: f32) -> Rgb {
        let a = amount.clamp(0.0, 1.0);
        let lerp = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * a) as u8;
        Rgb::new(
            lerp(self.r, other.r),
            lerp(self.g, other.g),
            lerp(self.b, other.b),
        )
    }
}

/// The palette a radio's console uses.
///
/// Six colours by role. A renderer derives the rest — panel fills, dimmed
/// text, pressed states — by mixing, so a radio describes its front panel
/// rather than every shade a console might need.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Theme {
    /// Behind everything. The case, in effect.
    pub background: Rgb,
    /// A panel's own fill, against the background.
    pub panel: Rgb,
    /// Text and instrument marks: the display's own colour.
    pub ink: Rgb,
    /// A value that has been set, or a filter edge. What the rig
    /// highlights.
    pub accent: Rgb,
    /// A meter's fill and a spectrum's trace.
    pub signal: Rgb,
    /// Transmit, and anything that should stop somebody.
    pub warning: Rgb,
}

impl Theme {
    /// Text that is present but not the point.
    pub fn dim(&self) -> Rgb {
        self.ink.mix(self.background, 0.45)
    }

    /// Text for something absent — a meter with no reading, a field not
    /// read yet. Deliberately further down than `dim`: "off" and "unknown"
    /// are different states and an operator should be able to tell.
    pub fn absent(&self) -> Rgb {
        self.ink.mix(self.background, 0.72)
    }

    /// A rule between panels.
    pub fn line(&self) -> Rgb {
        self.ink.mix(self.background, 0.80)
    }

    /// A panel that sits behind another — a meter trough, a scope ground.
    pub fn trough(&self) -> Rgb {
        self.panel.mix(self.background, 0.5)
    }

    /// The console's own default, for a server that publishes no theme.
    ///
    /// The instrument-panel look this console shipped with, so an older
    /// server's console does not change under its operator.
    pub const fn console_default() -> Self {
        Self {
            background: Rgb::hex(0x06080b),
            panel: Rgb::hex(0x0d1115),
            ink: Rgb::hex(0xc6d3dc),
            accent: Rgb::hex(0xe6ab44),
            signal: Rgb::hex(0x4ec97e),
            warning: Rgb::hex(0xe2543c),
        }
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self::console_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_is_the_way_a_palette_is_written_down() {
        assert_eq!(Rgb::hex(0xe6ab44), Rgb::new(0xe6, 0xab, 0x44));
        assert_eq!(Rgb::hex(0x000000), Rgb::new(0, 0, 0));
        assert_eq!(Rgb::hex(0xffffff), Rgb::new(255, 255, 255));
    }

    #[test]
    fn the_derived_shades_stay_between_ink_and_background() {
        // A renderer mixes rather than a radio enumerating twenty colours.
        // What must hold is that the derived shades are legible against
        // the panel they sit on, which starts with them being between the
        // two colours they were mixed from.
        for theme in [Theme::console_default(), sunlit()] {
            for shade in [theme.dim(), theme.absent()] {
                for channel in [
                    (shade.r, theme.ink.r, theme.background.r),
                    (shade.g, theme.ink.g, theme.background.g),
                    (shade.b, theme.ink.b, theme.background.b),
                ] {
                    let (s, a, b) = channel;
                    let (lo, hi) = (a.min(b), a.max(b));
                    assert!((lo..=hi).contains(&s), "{s} not within {lo}..={hi}");
                }
            }
        }
    }

    #[test]
    fn absent_is_further_down_than_dim() {
        // "Off" and "not read yet" are different states, and an operator
        // who cannot tell them apart is being told the radio is off when
        // it may only be quiet.
        let t = Theme::console_default();
        let brightness = |c: Rgb| u32::from(c.r) + u32::from(c.g) + u32::from(c.b);
        assert!(brightness(t.absent()) < brightness(t.dim()));
    }

    #[test]
    fn a_theme_survives_the_wire() {
        let t = sunlit();
        let json = serde_json::to_string(&t).unwrap();
        assert_eq!(serde_json::from_str::<Theme>(&json).unwrap(), t);
    }

    #[test]
    fn two_radios_can_look_different() {
        assert_ne!(Theme::console_default(), sunlit());
    }

    /// A deliberately unlike palette, for the tests above.
    fn sunlit() -> Theme {
        Theme {
            background: Rgb::hex(0x1a1408),
            panel: Rgb::hex(0x241c0c),
            ink: Rgb::hex(0xffb547),
            accent: Rgb::hex(0xfff0c0),
            signal: Rgb::hex(0xffd070),
            warning: Rgb::hex(0xff5533),
        }
    }
}
