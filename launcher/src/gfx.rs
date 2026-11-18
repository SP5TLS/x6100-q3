// SPDX-License-Identifier: MIT
//! Drawing: an 800x480 landscape canvas, shown on the framebuffer turned
//! for the X6100's panel (480x800, mounted sideways).

use crate::font::{Font, CW};
use std::fs::{File, OpenOptions};
use std::io;
use std::os::unix::fs::FileExt;

pub const W: i32 = 800;
pub const H: i32 = 480;

pub struct Canvas {
    pub px: Vec<u32>,
}

impl Canvas {
    pub fn new() -> Canvas {
        Canvas {
            px: vec![0; (W * H) as usize],
        }
    }

    pub fn clear(&mut self, c: u32) {
        self.px.fill(c);
    }

    pub fn rect(&mut self, x: i32, y: i32, w: i32, h: i32, c: u32) {
        let (x0, y0) = (x.max(0), y.max(0));
        let (x1, y1) = ((x + w).min(W), (y + h).min(H));
        for yy in y0..y1 {
            let row = (yy * W) as usize;
            self.px[row + x0 as usize..row + x1.max(x0) as usize].fill(c);
        }
    }

    /// A rectangle outline, `t` pixels thick.
    pub fn frame(&mut self, x: i32, y: i32, w: i32, h: i32, t: i32, c: u32) {
        self.rect(x, y, w, t, c);
        self.rect(x, y + h - t, w, t, c);
        self.rect(x, y, t, h, c);
        self.rect(x + w - t, y, t, h, c);
    }

    /// Text with its cell's top-left corner at (x, y), `scale` times the
    /// font's 10x20 cell; returns the width drawn.
    pub fn text(&mut self, font: &Font, x: i32, y: i32, s: &str, c: u32, scale: i32) -> i32 {
        let mut cx = x;
        for ch in s.chars() {
            let g = font.glyph(ch);
            for (row, bits) in g.iter().enumerate() {
                if *bits == 0 {
                    continue;
                }
                for col in 0..CW {
                    if bits & (0x8000 >> col) != 0 {
                        self.rect(cx + col * scale, y + row as i32 * scale, scale, scale, c);
                    }
                }
            }
            cx += CW * scale;
        }
        cx - x
    }

    /// Text cut to `max_w` pixels, with an ellipsis when it did not fit.
    #[allow(clippy::too_many_arguments)]
    pub fn text_fit(&mut self, font: &Font, x: i32, y: i32, s: &str, c: u32, scale: i32, max_w: i32) -> i32 {
        let fit = fit(s, scale, max_w);
        self.text(font, x, y, &fit, c, scale)
    }
}

pub fn text_w(s: &str, scale: i32) -> i32 {
    s.chars().count() as i32 * CW * scale
}

/// `s` shortened (with '…') to at most `max_w` pixels at `scale`.
pub fn fit(s: &str, scale: i32, max_w: i32) -> String {
    let max = (max_w / (CW * scale)).max(0) as usize;
    if s.chars().count() <= max {
        return s.to_string();
    }
    if max == 0 {
        return String::new();
    }
    let mut t: String = s.chars().take(max - 1).collect();
    t.push('…');
    t
}

/// The framebuffer, written whole on every frame (no mmap).
pub struct Fb {
    file: File,
    xres: usize,
    yres: usize,
    bpp: usize,
    stride: usize,
    offset: u64,
    rotate: u32,
    buf: Vec<u8>,
}

impl Fb {
    /// `rotate`: how the landscape picture is turned onto the device
    /// (270 for the X6100's panel, as its device tree and fbcon). A plain
    /// file (tests) is taken as a 480x800 XRGB8888 device.
    pub fn open(path: &str, rotate: u32) -> io::Result<Fb> {
        let file = OpenOptions::new().read(true).write(true).open(path)?;
        let mut fb = Fb {
            file,
            xres: 480,
            yres: 800,
            bpp: 32,
            stride: 480 * 4,
            offset: 0,
            rotate,
            buf: Vec::new(),
        };
        fb.refresh_geometry(path);
        Ok(fb)
    }

    /// Read the mode again (after the game, which may have changed it).
    pub fn refresh_geometry(&mut self, path: &str) {
        if let Some((x, y, bpp, yoff)) = crate::sys::fb_var_info(&self.file) {
            self.xres = x;
            self.yres = y;
            self.bpp = bpp;
            let name = path.rsplit('/').next().unwrap_or("fb0");
            self.stride = std::fs::read_to_string(format!("/sys/class/graphics/{name}/stride"))
                .ok()
                .and_then(|s| s.trim().parse().ok())
                .unwrap_or(x * bpp / 8);
            self.offset = (yoff * self.stride) as u64;
        }
        // Landscape devices (a desktop) need no turning.
        if self.xres >= self.yres {
            self.rotate = 0;
        }
        self.buf = vec![0; self.stride * self.yres];
    }

    pub fn present(&mut self, c: &Canvas) -> io::Result<()> {
        let (xres, yres, stride) = (self.xres, self.yres, self.stride);
        let bpp = self.bpp;
        let rot = self.rotate;
        for py in 0..yres {
            let line = &mut self.buf[py * stride..];
            for px in 0..xres {
                // Device pixel (px, py) shows canvas pixel (x, y).
                let (x, y) = match rot {
                    90 => (py as i32, (xres - 1 - px) as i32),
                    270 => ((yres - 1 - py) as i32, px as i32),
                    _ => (px as i32, py as i32),
                };
                let v = if x < W && y < H {
                    c.px[(y * W + x) as usize]
                } else {
                    0
                };
                if bpp == 32 {
                    line[px * 4..px * 4 + 4].copy_from_slice(&v.to_le_bytes());
                } else {
                    let r = (v >> 19) & 0x1f;
                    let g = (v >> 10) & 0x3f;
                    let b = (v >> 3) & 0x1f;
                    let p = ((r << 11) | (g << 5) | b) as u16;
                    line[px * 2..px * 2 + 2].copy_from_slice(&p.to_le_bytes());
                }
            }
        }
        self.file.write_all_at(&self.buf, self.offset)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_shortens_with_an_ellipsis() {
        assert_eq!(fit("abcdef", 1, 60), "abcdef");
        assert_eq!(fit("abcdefg", 1, 60), "abcde…");
        assert_eq!(fit("abc", 2, 10), "");
    }

    #[test]
    fn rotation_270_matches_the_panel() {
        // Logical (x, y) -> device (y, 799 - x), as x6100-splash and fbcon.
        let path = std::env::temp_dir().join(format!("x6100-fb-test-{}", std::process::id()));
        std::fs::write(&path, vec![0u8; 480 * 800 * 4]).unwrap();
        let mut fb = Fb::open(path.to_str().unwrap(), 270).unwrap();
        let mut c = Canvas::new();
        c.px[(10 * W + 3) as usize] = 0x00ff_0000; // x 3, y 10
        fb.present(&c).unwrap();
        let data = std::fs::read(&path).unwrap();
        let (px, py) = (10usize, 799 - 3usize);
        let o = (py * 480 + px) * 4;
        assert_eq!(&data[o..o + 4], &[0, 0, 0xff, 0]);
        std::fs::remove_file(path).unwrap();
    }
}
