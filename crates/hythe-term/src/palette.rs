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
            fg: c(0xdc, 0xe0, 0xe5),
            bg: c(0x26, 0x2a, 0x32),
            cursor: c(0x74, 0xad, 0xe8),
            ansi: [
                c(0x3f, 0x44, 0x51),
                c(0xe0, 0x6c, 0x75),
                c(0x98, 0xc3, 0x79),
                c(0xe5, 0xc0, 0x7b),
                c(0x61, 0xaf, 0xef),
                c(0xc6, 0x78, 0xdd),
                c(0x56, 0xb6, 0xc2),
                c(0xab, 0xb2, 0xbf),
                c(0x5c, 0x63, 0x70),
                c(0xff, 0x7b, 0x86),
                c(0xb1, 0xe0, 0x8c),
                c(0xf5, 0xd0, 0x8b),
                c(0x7c, 0xc3, 0xff),
                c(0xe0, 0x8c, 0xf5),
                c(0x6c, 0xd0, 0xdd),
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
