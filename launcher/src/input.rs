// SPDX-License-Identifier: MIT
//! Input: every /dev/input/event* is read (non-exclusively, so the game
//! sees the same devices), and turned into a few actions. The radio's own
//! controls are known by their device names from the device tree:
//! "keypad" (BTN_TRIGGER_HAPPYn), "encoder-vfo/vol/mfk" (REL axes 0/1/2),
//! "axp20x-pek" (KEY_POWER). Anything else is a keyboard or a mouse.

use std::collections::HashSet;
use std::fs::File;
use std::io::Read;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Knob {
    Vfo,
    Vol,
    Mfk,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Act {
    Up,
    Down,
    Left,
    Right,
    PageUp,
    PageDown,
    Home,
    End,
    /// Enter on a keyboard, a left click.
    Enter,
    /// A knob pushed (VOL or MFK).
    Push,
    /// Esc, a right click.
    Back,
    Backspace,
    Delete,
    Tab,
    Char(char),
    /// One of the five keys under the screen, 0 = leftmost.
    Soft(u8),
    Turn(Knob, i32),
    PowerDown,
    PowerUp,
}

const EV_KEY: u16 = 0x01;
const EV_REL: u16 = 0x02;
const REL_WHEEL: u16 = 0x08;
const KEY_POWER: u16 = 116;
const BTN_LEFT: u16 = 0x110;
const BTN_RIGHT: u16 = 0x111;
const BTN_TRIGGER_HAPPY1: u16 = 0x2c0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Keypad,
    Encoder(Knob),
    Other,
}

fn kind_of(name: &str) -> Kind {
    match name {
        "keypad" => Kind::Keypad,
        "encoder-vfo" => Kind::Encoder(Knob::Vfo),
        "encoder-vol" => Kind::Encoder(Knob::Vol),
        "encoder-mfk" => Kind::Encoder(Knob::Mfk),
        _ => Kind::Other,
    }
}

/// Keyboard modifiers, shared by all keyboards.
static SHIFT: AtomicBool = AtomicBool::new(false);
static CAPS: AtomicBool = AtomicBool::new(false);
static CTRL: AtomicBool = AtomicBool::new(false);

/// Start reading input devices; actions go to `tx` (wrapped by `wrap`).
pub fn start<M: Send + 'static>(tx: Sender<M>, wrap: fn(Act) -> M) {
    let open: Arc<Mutex<HashSet<PathBuf>>> = Arc::default();
    std::thread::Builder::new()
        .name("input-scan".into())
        .spawn(move || loop {
            if let Ok(dir) = std::fs::read_dir("/dev/input") {
                let mut paths: Vec<PathBuf> = dir
                    .filter_map(|e| e.ok().map(|e| e.path()))
                    .filter(|p| {
                        p.file_name()
                            .and_then(|n| n.to_str())
                            .is_some_and(|n| n.starts_with("event"))
                    })
                    .collect();
                paths.sort();
                for p in paths {
                    if crate::sys::lock(&open).contains(&p) {
                        continue;
                    }
                    let Ok(f) = File::open(&p) else { continue };
                    let n = p.file_name().unwrap().to_string_lossy().to_string();
                    let name = std::fs::read_to_string(format!("/sys/class/input/{n}/device/name"))
                        .unwrap_or_default()
                        .trim()
                        .to_string();
                    eprintln!("launcher: input {} ({name})", p.display());
                    crate::sys::lock(&open).insert(p.clone());
                    let (tx, open) = (tx.clone(), open.clone());
                    let _ = std::thread::Builder::new()
                        .name(format!("input-{n}"))
                        .spawn(move || {
                            read_device(f, kind_of(&name), &tx, wrap);
                            crate::sys::lock(&open).remove(&p);
                        });
                }
            }
            std::thread::sleep(Duration::from_secs(2));
        })
        .expect("input thread");
}

fn read_device<M>(mut f: File, kind: Kind, tx: &Sender<M>, wrap: fn(Act) -> M) {
    // struct input_event: struct timeval (two longs), u16 type, u16 code,
    // s32 value: 16 bytes on 32-bit ARM, 24 on 64-bit.
    let size = 2 * std::mem::size_of::<std::os::raw::c_long>() + 8;
    let mut buf = vec![0u8; size * 64];
    loop {
        let n = match f.read(&mut buf) {
            Ok(0) | Err(_) => return,
            Ok(n) => n,
        };
        for ev in buf[..n].chunks_exact(size) {
            let o = size - 8;
            let ty = u16::from_ne_bytes([ev[o], ev[o + 1]]);
            let code = u16::from_ne_bytes([ev[o + 2], ev[o + 3]]);
            let value = i32::from_ne_bytes([ev[o + 4], ev[o + 5], ev[o + 6], ev[o + 7]]);
            if let Some(a) = translate(kind, ty, code, value) {
                if tx.send(wrap(a)).is_err() {
                    return;
                }
            }
        }
    }
}

fn translate(kind: Kind, ty: u16, code: u16, value: i32) -> Option<Act> {
    match (ty, kind) {
        (EV_REL, Kind::Encoder(k)) if value != 0 => Some(Act::Turn(k, value)),
        (EV_REL, Kind::Other) if code == REL_WHEEL && value != 0 => {
            Some(if value > 0 { Act::Up } else { Act::Down })
        }
        (EV_KEY, _) if code == KEY_POWER => match value {
            1 => Some(Act::PowerDown),
            0 => Some(Act::PowerUp),
            _ => None,
        },
        (EV_KEY, Kind::Keypad) if value == 1 => keypad(code),
        (EV_KEY, Kind::Other) => keyboard(code, value),
        _ => None,
    }
}

/// The radio's keys: the five under the screen and the knob pushes.
fn keypad(code: u16) -> Option<Act> {
    match code.checked_sub(BTN_TRIGGER_HAPPY1)? + 1 {
        13 => Some(Act::Soft(0)),
        14 => Some(Act::Soft(1)),
        15 => Some(Act::Soft(2)),
        19 => Some(Act::Soft(3)),
        20 => Some(Act::Soft(4)),
        21 | 27 => Some(Act::Push),
        _ => None,
    }
}

fn keyboard(code: u16, value: i32) -> Option<Act> {
    match code {
        42 | 54 => {
            SHIFT.store(value != 0, Ordering::Relaxed);
            return None;
        }
        29 | 97 => {
            CTRL.store(value != 0, Ordering::Relaxed);
            return None;
        }
        58 if value == 1 => {
            CAPS.fetch_xor(true, Ordering::Relaxed);
            return None;
        }
        _ => {}
    }
    // Presses and auto-repeat; mouse buttons on press only.
    match (code, value) {
        (BTN_LEFT, 1) => return Some(Act::Enter),
        (BTN_RIGHT, 1) => return Some(Act::Back),
        (_, 1 | 2) => {}
        _ => return None,
    }
    let a = match code {
        1 => Act::Back,
        14 => Act::Backspace,
        15 => Act::Tab,
        28 | 96 => Act::Enter,
        103 => Act::Up,
        108 => Act::Down,
        105 => Act::Left,
        106 => Act::Right,
        104 => Act::PageUp,
        109 => Act::PageDown,
        102 => Act::Home,
        107 => Act::End,
        111 => Act::Delete,
        // F1..F5 stand in for the soft keys.
        59..=63 => Act::Soft((code - 59) as u8),
        _ => {
            if CTRL.load(Ordering::Relaxed) {
                return None;
            }
            let shift = SHIFT.load(Ordering::Relaxed);
            let c = us_char(code, shift)?;
            let c = if CAPS.load(Ordering::Relaxed) && c.is_ascii_alphabetic() {
                if shift {
                    c.to_ascii_lowercase()
                } else {
                    c.to_ascii_uppercase()
                }
            } else {
                c
            };
            Act::Char(c)
        }
    };
    Some(a)
}

/// US keyboard layout.
fn us_char(code: u16, shift: bool) -> Option<char> {
    const ROWS: [(u16, &str, &str); 4] = [
        (2, "1234567890-=", "!@#$%^&*()_+"),
        (16, "qwertyuiop[]", "QWERTYUIOP{}"),
        (30, "asdfghjkl;'`", "ASDFGHJKL:\"~"),
        (43, "\\zxcvbnm,./", "|ZXCVBNM<>?"),
    ];
    if code == 57 {
        return Some(' ');
    }
    // Keypad digits and operators (without NumLock handling).
    let kp = match code {
        71 => Some('7'),
        72 => Some('8'),
        73 => Some('9'),
        74 => Some('-'),
        75 => Some('4'),
        76 => Some('5'),
        77 => Some('6'),
        78 => Some('+'),
        79 => Some('1'),
        80 => Some('2'),
        81 => Some('3'),
        82 => Some('0'),
        83 => Some('.'),
        55 => Some('*'),
        98 => Some('/'),
        _ => None,
    };
    if kp.is_some() {
        return kp;
    }
    for (first, plain, shifted) in ROWS {
        let i = code.checked_sub(first)? as usize;
        if i < plain.len() {
            let s = if shift { shifted } else { plain };
            return s.chars().nth(i);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_and_knobs() {
        assert_eq!(translate(Kind::Keypad, EV_KEY, BTN_TRIGGER_HAPPY1 + 12, 1), Some(Act::Soft(0)));
        assert_eq!(translate(Kind::Keypad, EV_KEY, BTN_TRIGGER_HAPPY1 + 19, 1), Some(Act::Soft(4)));
        assert_eq!(translate(Kind::Keypad, EV_KEY, BTN_TRIGGER_HAPPY1 + 26, 1), Some(Act::Push));
        assert_eq!(translate(Kind::Keypad, EV_KEY, BTN_TRIGGER_HAPPY1 + 26, 0), None);
        assert_eq!(translate(Kind::Encoder(Knob::Vfo), EV_REL, 0, -1), Some(Act::Turn(Knob::Vfo, -1)));
        // A mouse moving is not a knob.
        assert_eq!(translate(Kind::Other, EV_REL, 0, 5), None);
        assert_eq!(translate(Kind::Other, EV_REL, REL_WHEEL, -1), Some(Act::Down));
        assert_eq!(translate(Kind::Other, EV_KEY, KEY_POWER, 1), Some(Act::PowerDown));
    }

    #[test]
    fn us_layout() {
        assert_eq!(us_char(16, false), Some('q'));
        assert_eq!(us_char(16, true), Some('Q'));
        assert_eq!(us_char(2, true), Some('!'));
        assert_eq!(us_char(53, false), Some('/'));
        assert_eq!(us_char(43, false), Some('\\'));
        assert_eq!(us_char(41, true), Some('~'));
        assert_eq!(us_char(57, false), Some(' '));
        assert_eq!(us_char(1, false), None);
    }
}
