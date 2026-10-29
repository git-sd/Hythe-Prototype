/// 24-bit colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    /// Move `self` towards `other` by `t` (0.0 keeps `self`, 1.0 gives `other`).
    pub fn mix(self, other: Rgb, t: f32) -> Rgb {
        let m = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t).round() as u8;
        Rgb(m(self.0, other.0), m(self.1, other.1), m(self.2, other.2))
    }
}

/// Colours used to resolve the terminal's named and indexed colours.
#[derive(Debug, Clone)]
pub struct Palette {
    pub fg: Rgb,
    pub bg: Rgb,
    pub cursor: Rgb,
    /// ANSI colours 0-7 followed by their bright variants 8-15.
    pub ansi: [Rgb; 16],
}

impl Default for Palette {
    fn default() -> Self {
        let c = |r, g, b| Rgb(r, g, b);
        Self {
            fg: c(0xd5, 0xd9, 0xe0),
            bg: c(0x17, 0x1a, 0x1f),
            cursor: c(0x3f, 0xc1, 0xb0),
            ansi: [
                c(0x2a, 0x2f, 0x38),
                c(0xe8, 0x7a, 0x8a),
                c(0xa5, 0xc2, 0x61),
                c(0xf0, 0xc6, 0x74),
                c(0x6c, 0xb6, 0xff),
                c(0xc7, 0x92, 0xea),
                c(0x7f, 0xd6, 0xd0),
                c(0xd5, 0xd9, 0xe0),
                c(0x5f, 0x69, 0x78),
                c(0xff, 0x9e, 0xab),
                c(0xc3, 0xe0, 0x7f),
                c(0xff, 0xdc, 0x95),
                c(0x95, 0xcf, 0xff),
                c(0xdf, 0xb2, 0xff),
                c(0xa2, 0xec, 0xe6),
                c(0xff, 0xff, 0xff),
            ],
        }
    }
}

impl Palette {
    /// Colour for an xterm 256-colour index.
    pub fn indexed(&self, i: u8) -> Rgb {
        match i {
            0..=15 => self.ansi[i as usize],
            16..=231 => {
                let n = i - 16;
                let level = |v: u8| if v == 0 { 0 } else { 55 + 40 * v };
                Rgb(level(n / 36), level((n / 6) % 6), level(n % 6))
            }
            232..=255 => {
                let v = 8 + 10 * (i - 232);
                Rgb(v, v, v)
            }
        }
    }

    pub(crate) fn dim(&self, c: Rgb) -> Rgb {
        c.mix(self.bg, 0.4)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xterm_cube_and_grayscale() {
        let p = Palette::default();
        assert_eq!(p.indexed(16), Rgb(0, 0, 0));
        assert_eq!(p.indexed(196), Rgb(255, 0, 0));
        assert_eq!(p.indexed(231), Rgb(255, 255, 255));
        assert_eq!(p.indexed(232), Rgb(8, 8, 8));
        assert_eq!(p.indexed(255), Rgb(238, 238, 238));
        assert_eq!(p.indexed(1), p.ansi[1]);
    }
}
