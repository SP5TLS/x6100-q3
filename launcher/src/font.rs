// SPDX-License-Identifier: MIT
//! The X11 misc-fixed 10x20 font (public domain, see fonts/README.md),
//! parsed from the BDF subset compiled into the binary.

use std::collections::HashMap;

/// Cell width and height in pixels (every glyph fills one cell).
pub const CW: i32 = 10;
pub const CH: i32 = 20;

pub type Glyph = [u16; CH as usize];

pub struct Font {
    glyphs: HashMap<char, Glyph>,
    fallback: Glyph,
}

static BDF: &str = include_str!("../fonts/10x20.bdf");

impl Font {
    pub fn load() -> Font {
        let mut glyphs = HashMap::new();
        let mut enc: Option<u32> = None;
        let mut rows: Vec<u16> = Vec::with_capacity(CH as usize);
        let mut bitmap = false;
        for line in BDF.lines() {
            if let Some(e) = line.strip_prefix("ENCODING ") {
                enc = e.trim().parse().ok();
                rows.clear();
            } else if line == "BITMAP" {
                bitmap = true;
            } else if line == "ENDCHAR" {
                bitmap = false;
                if let Some(c) = enc.and_then(char::from_u32) {
                    if rows.len() == CH as usize {
                        let mut g = [0u16; CH as usize];
                        g.copy_from_slice(&rows);
                        glyphs.insert(c, g);
                    }
                }
                rows.clear();
                enc = None;
            } else if bitmap {
                // One row: 16 bits, the leftmost pixel in the top bit.
                rows.push(u16::from_str_radix(line.trim(), 16).unwrap_or(0));
            }
        }
        let fallback = glyphs
            .get(&'\u{fffd}')
            .or_else(|| glyphs.get(&'?'))
            .copied()
            .unwrap_or([0; CH as usize]);
        Font { glyphs, fallback }
    }

    pub fn glyph(&self, c: char) -> &Glyph {
        self.glyphs.get(&c).unwrap_or(&self.fallback)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_and_latin_glyphs_are_there() {
        let f = Font::load();
        for c in ['A', 'z', '0', '~', 'ą', 'Ł', 'ж', '▶', '✓'] {
            assert!(f.glyphs.contains_key(&c), "{c}");
        }
        // 'A' has pixels, ' ' has none.
        assert!(f.glyph('A').iter().any(|r| *r != 0));
        assert!(f.glyph(' ').iter().all(|r| *r == 0));
    }
}
