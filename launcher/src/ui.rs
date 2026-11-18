// SPDX-License-Identifier: MIT
//! The on-screen menu: play, Wi-Fi, Bluetooth, power off. Driven by the
//! radio's knobs and keys (VFO/MFK/VOL turn to move, VOL or MFK push to
//! choose, the five keys under the screen as labelled) or by a keyboard
//! and mouse.

use crate::font::Font;
use crate::gfx::{fit, text_w, Canvas, H, W};
use crate::input::{Act, Knob};
use crate::net::{self, BtDevice, Network};
use crate::state::{Ctx, Shared};
use crate::sys::lock;
use std::time::{Duration, Instant};

const BG: u32 = 0x10141a;
const BAR: u32 = 0x1d2633;
const TEXT: u32 = 0xe8eaed;
const DIM: u32 = 0x8b97a5;
const OFF: u32 = 0x58626e;
const SEL: u32 = 0x2f6fd6;
const OK: u32 = 0x3fb950;
const ERR: u32 = 0xff6b5e;
const BUSY: u32 = 0xe3b341;

const HEAD_H: i32 = 56;
const LIST_Y: i32 = 64;
const ROW_H: i32 = 46;
const STATUS_Y: i32 = 392;
const FOOT_Y: i32 = 432;
const ROWS: usize = ((STATUS_Y - LIST_Y) / ROW_H) as usize;

/// Characters offered for a password with the knobs, in four groups
/// (MFK jumps between them).
const CHARSET: &str =
    "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789 !\"#$%&'()*+,-./:;<=>?@[\\]^_`{|}~";
const GROUPS: [usize; 4] = [0, 26, 52, 62];

#[derive(Clone, Copy, PartialEq, Eq)]
enum K {
    Home,
    Wifi,
    Password,
    Bt,
    BtDevice,
    Power,
}

enum Screen {
    Home { sel: usize },
    Wifi { sel: usize },
    Password { ssid: String, saved: bool, text: String, pick: usize, show: bool },
    Bt { sel: usize },
    BtDevice { dev: BtDevice, sel: usize },
    Power { sel: usize },
}

#[derive(Clone)]
enum Do {
    Nothing(String),
    Play(String),
    Wifi,
    Bt,
    Power,
    WifiToggle(bool),
    WifiScan,
    WifiPick(Network, bool),
    BtToggle(bool),
    BtScan,
    BtOpen(BtDevice),
    BtPair(BtDevice),
    BtCmd(&'static str, String),
    PowerOff,
    Back,
}

struct Item {
    label: String,
    value: String,
    enabled: bool,
    act: Do,
}

fn item(label: impl Into<String>, value: impl Into<String>, act: Do) -> Item {
    Item {
        label: label.into(),
        value: value.into(),
        enabled: true,
        act,
    }
}

/// What the main loop has to do for the menu.
pub enum Out {
    None,
    Play(String),
    PowerOff,
}

pub struct Ui {
    font: Font,
    pub canvas: Canvas,
    stack: Vec<Screen>,
    spin: usize,
}

impl Ui {
    pub fn new() -> Ui {
        Ui {
            font: Font::load(),
            canvas: Canvas::new(),
            stack: vec![Screen::Home { sel: 0 }],
            spin: 0,
        }
    }

    fn top(&mut self) -> &mut Screen {
        self.stack.last_mut().unwrap()
    }

    fn back(&mut self) {
        if self.stack.len() > 1 {
            self.stack.pop();
        }
    }

    pub fn home(&mut self) {
        self.stack.truncate(1);
    }

    /// A short press of the power key: the Power screen, or, when it is
    /// already showing, true (switch off).
    pub fn power_key(&mut self) -> bool {
        if self.kind() == K::Power {
            return true;
        }
        self.stack.push(Screen::Power { sel: 1 });
        false
    }

    /// Whether something on screen moves (redraw often).
    pub fn animating(s: &Shared) -> bool {
        s.busy.is_some() || s.upload.is_some()
    }

    // ------------------------------------------------------------------
    // Menus

    fn items(&self, s: &Shared) -> Vec<Item> {
        match self.stack.last().unwrap() {
            Screen::Home { .. } => home_items(s),
            Screen::Wifi { .. } => wifi_items(s),
            Screen::Bt { .. } => bt_items(s),
            Screen::BtDevice { dev, .. } => {
                let connected = s
                    .bt_paired
                    .iter()
                    .find(|d| d.mac == dev.mac)
                    .map_or(dev.connected, |d| d.connected);
                vec![
                    if connected {
                        item("Disconnect", "", Do::BtCmd("disconnect", dev.mac.clone()))
                    } else {
                        item("Connect", "", Do::BtCmd("connect", dev.mac.clone()))
                    },
                    item("Forget this device", "", Do::BtCmd("forget", dev.mac.clone())),
                    item("Back", "", Do::Back),
                ]
            }
            Screen::Power { .. } => vec![
                item("Switch off now", "", Do::PowerOff),
                item("Cancel", "", Do::Back),
            ],
            Screen::Password { .. } => Vec::new(),
        }
    }

    fn sel_mut(&mut self) -> Option<&mut usize> {
        match self.top() {
            Screen::Home { sel }
            | Screen::Wifi { sel }
            | Screen::Bt { sel }
            | Screen::BtDevice { sel, .. }
            | Screen::Power { sel } => Some(sel),
            Screen::Password { .. } => None,
        }
    }

    fn soft_labels(&self) -> [&'static str; 5] {
        match self.stack.last().unwrap() {
            Screen::Home { .. } => ["Wi-Fi", "Bluetooth", "", "Power off", "OK"],
            Screen::Wifi { .. } => ["Back", "Scan", "Forget", "On/Off", "OK"],
            Screen::Bt { .. } => ["Back", "Scan", "", "On/Off", "OK"],
            Screen::BtDevice { .. } => ["Back", "", "", "", "OK"],
            Screen::Power { .. } => ["Cancel", "", "", "", "OK"],
            Screen::Password { show, .. } => {
                ["Cancel", "Delete", "Space", if *show { "Hide" } else { "Show" }, "Connect"]
            }
        }
    }

    fn kind(&self) -> K {
        match self.stack.last().unwrap() {
            Screen::Home { .. } => K::Home,
            Screen::Wifi { .. } => K::Wifi,
            Screen::Password { .. } => K::Password,
            Screen::Bt { .. } => K::Bt,
            Screen::BtDevice { .. } => K::BtDevice,
            Screen::Power { .. } => K::Power,
        }
    }

    pub fn handle(&mut self, a: Act, ctx: &Ctx) -> Out {
        let s = lock(&ctx.shared).clone();
        if self.kind() == K::Password {
            return self.handle_password(a, ctx);
        }
        let items = self.items(&s);
        let n = items.len() as i64;
        let mut sel = self.sel_mut().map_or(0, |x| *x) as i64;
        let nav = match a {
            Act::Up | Act::Left => Some(-1),
            Act::Down | Act::Right | Act::Tab => Some(1),
            Act::PageUp => Some(-(ROWS as i64)),
            Act::PageDown => Some(ROWS as i64),
            Act::Home => Some(-n),
            Act::End => Some(n),
            Act::Turn(_, d) => Some(d as i64),
            _ => None,
        };
        if let Some(d) = nav {
            sel += d;
        }
        let sel = sel.clamp(0, (n - 1).max(0)) as usize;
        if let Some(x) = self.sel_mut() {
            *x = sel;
        }
        match a {
            Act::Enter | Act::Push | Act::Soft(4) | Act::Char(' ') => {
                if let Some(it) = items.get(sel) {
                    return self.choose(it, &s, ctx);
                }
            }
            Act::Back | Act::Backspace | Act::Soft(0) if self.stack.len() > 1 => self.back(),
            Act::Soft(k) => self.soft(k, items.get(sel).map(|i| i.act.clone()), &s, ctx),
            _ => {}
        }
        Out::None
    }

    fn soft(&mut self, k: u8, current: Option<Do>, s: &Shared, ctx: &Ctx) {
        match (self.kind(), k) {
            (K::Home, 0) => self.stack.push(Screen::Wifi { sel: 0 }),
            (K::Home, 1) => self.stack.push(Screen::Bt { sel: 0 }),
            (K::Home, 3) => self.stack.push(Screen::Power { sel: 1 }),
            (K::Wifi, 1) => wifi_scan(ctx),
            (K::Wifi, 2) => match current {
                Some(Do::WifiPick(n, true)) => {
                    let name = n.ssid.clone();
                    ctx.spawn_job("Forgetting the network", move || net::wifi_forget(&name), |c, m| {
                        c.note(m, false)
                    });
                }
                _ => ctx.note("Choose a saved network to forget", true),
            },
            (K::Wifi, 3) => {
                let on = !s.wifi.enabled;
                ctx.spawn_job("Switching Wi-Fi", move || net::wifi_enable(on), |c, m| c.note(m, false));
            }
            (K::Bt, 1) => bt_scan(ctx),
            (K::Bt, 3) => {
                let on = !s.bt.powered;
                ctx.spawn_job(
                    "Switching Bluetooth",
                    move || net::bt(if on { "on" } else { "off" }, None),
                    |c, m| c.note(m, false),
                );
            }
            _ => {}
        }
    }

    fn choose(&mut self, it: &Item, s: &Shared, ctx: &Ctx) -> Out {
        match &it.act {
            Do::Nothing(why) => {
                if !why.is_empty() {
                    ctx.note(why.clone(), true);
                }
            }
            Do::Play(g) => {
                let game = s.games.iter().find(|x| &x.id == g);
                match game {
                    Some(x) if x.missing.is_empty() => return Out::Play(g.clone()),
                    Some(x) => ctx.note(
                        format!("{} needs {} — upload on the web page", x.title, x.missing.join(" ")),
                        true,
                    ),
                    None => {}
                }
            }
            Do::Wifi => self.stack.push(Screen::Wifi { sel: 0 }),
            Do::Bt => self.stack.push(Screen::Bt { sel: 0 }),
            Do::Power => self.stack.push(Screen::Power { sel: 1 }),
            Do::WifiToggle(on) => {
                let on = *on;
                ctx.spawn_job("Switching Wi-Fi", move || net::wifi_enable(on), |c, m| c.note(m, false));
            }
            Do::WifiScan => wifi_scan(ctx),
            Do::WifiPick(n, saved) => {
                if n.in_use {
                    ctx.note(format!("Connected to {} (Forget: third key)", n.ssid), false);
                } else if *saved || n.security.is_empty() {
                    let (ssid, saved) = (n.ssid.clone(), *saved);
                    ctx.spawn_job(
                        &format!("Connecting to {ssid}"),
                        move || net::wifi_connect(&ssid, None, saved),
                        |c, m| c.note(m, false),
                    );
                } else {
                    self.stack.push(Screen::Password {
                        ssid: n.ssid.clone(),
                        saved: false,
                        text: String::new(),
                        pick: 0,
                        show: true,
                    });
                }
            }
            Do::BtToggle(on) => {
                let on = *on;
                ctx.spawn_job("Switching Bluetooth", move || net::bt(if on { "on" } else { "off" }, None), |c, m| {
                    c.note(m, false)
                });
            }
            Do::BtScan => bt_scan(ctx),
            Do::BtOpen(d) => self.stack.push(Screen::BtDevice { dev: d.clone(), sel: 0 }),
            Do::BtPair(d) => {
                let (mac, name) = (d.mac.clone(), d.name.clone());
                let mac2 = mac.clone();
                ctx.spawn_job(
                    &format!("Pairing with {name}"),
                    move || net::bt("pair", Some(&mac)),
                    move |c, _| {
                        lock(&c.shared).bt_found.retain(|x| x.mac != mac2);
                        c.note(format!("Paired with {name}"), false)
                    },
                );
            }
            Do::BtCmd(cmd, mac) => {
                let (cmd, mac) = (*cmd, mac.clone());
                let what = match cmd {
                    "connect" => "Connecting",
                    "disconnect" => "Disconnecting",
                    _ => "Forgetting the device",
                };
                ctx.spawn_job(what, move || net::bt(cmd, Some(&mac)), |c, m| c.note(m, false));
                if cmd == "forget" {
                    self.back();
                }
            }
            Do::PowerOff => return Out::PowerOff,
            Do::Back => self.back(),
        }
        Out::None
    }

    fn handle_password(&mut self, a: Act, ctx: &Ctx) -> Out {
        let chars: Vec<char> = CHARSET.chars().collect();
        let n = chars.len() as i32;
        let mut leave = false;
        let mut connect = None;
        if let Some(Screen::Password { ssid, saved, text, pick, show }) = self.stack.last_mut() {
            let add = |text: &mut String, c: char| {
                if text.chars().count() < 63 {
                    text.push(c)
                }
            };
            match a {
                Act::Char(c) => add(text, c),
                Act::Turn(Knob::Mfk, d) => {
                    // Next or previous group (a-z, A-Z, 0-9, symbols).
                    let g = GROUPS.iter().rposition(|s| *s <= *pick).unwrap_or(0) as i32;
                    let g = (g + d.signum()).rem_euclid(GROUPS.len() as i32);
                    *pick = GROUPS[g as usize];
                }
                Act::Turn(_, d) => *pick = (*pick as i32 + d).rem_euclid(n) as usize,
                Act::Right | Act::Down => *pick = (*pick as i32 + 1).rem_euclid(n) as usize,
                Act::Left | Act::Up => *pick = (*pick as i32 - 1).rem_euclid(n) as usize,
                Act::Push => add(text, chars[*pick]),
                Act::Soft(1) | Act::Backspace | Act::Delete => {
                    text.pop();
                }
                Act::Soft(2) => add(text, ' '),
                Act::Soft(3) | Act::Tab => *show = !*show,
                Act::Soft(4) | Act::Enter => {
                    if text.len() < 8 {
                        ctx.note("A WPA password has at least 8 characters", true);
                    } else {
                        connect = Some((ssid.clone(), text.clone(), *saved));
                    }
                }
                Act::Soft(0) | Act::Back => leave = true,
                _ => {}
            }
        }
        if let Some((ssid, pw, saved)) = connect {
            ctx.spawn_job(
                &format!("Connecting to {ssid}"),
                move || net::wifi_connect(&ssid, Some(&pw), saved),
                |c, m| c.note(m, false),
            );
            leave = true;
        }
        if leave {
            self.back();
        }
        Out::None
    }

    // ------------------------------------------------------------------
    // Drawing

    pub fn draw(&mut self, ctx: &Ctx) {
        let s = lock(&ctx.shared).clone();
        self.spin = self.spin.wrapping_add(1);
        let kind = self.kind();
        let items = if kind == K::Password { Vec::new() } else { self.items(&s) };
        let sel = self
            .stack
            .last()
            .and_then(|sc| match sc {
                Screen::Home { sel }
                | Screen::Wifi { sel }
                | Screen::Bt { sel }
                | Screen::BtDevice { sel, .. }
                | Screen::Power { sel } => Some(*sel),
                Screen::Password { .. } => None,
            })
            .unwrap_or(0)
            .min(items.len().saturating_sub(1));
        let password = match self.stack.last() {
            Some(Screen::Password { ssid, text, pick, show, .. }) => {
                Some((ssid.clone(), text.clone(), *pick, *show))
            }
            _ => None,
        };
        let labels = self.soft_labels();
        let spin = ['◐', '◓', '◑', '◒'][self.spin % 4];
        let f = &self.font;
        let c = &mut self.canvas;
        c.clear(BG);
        if s.powering_off {
            centre(c, f, "Switching off…", 200, TEXT, 3);
            return;
        }
        if let Some((g, _)) = &s.playing {
            centre(c, f, &format!("Starting {}…", crate::games::title(g)), 180, TEXT, 2);
            centre(c, f, "Leave the game from its menu, or press the power key.", 240, DIM, 1);
            return;
        }

        // Header: title, Wi-Fi and Bluetooth state.
        c.rect(0, 0, W, HEAD_H, BAR);
        let title = match kind {
            K::Home => "x6100-q3",
            K::Wifi => "Wi-Fi",
            K::Password => "Wi-Fi password",
            K::Bt => "Bluetooth",
            K::BtDevice => "Bluetooth device",
            K::Power => "Power",
        };
        c.text(f, 16, 8, title, TEXT, 2);
        let (l1, l2) = header_status(&s);
        let x1 = W - 16 - text_w(&l1, 1);
        let x2 = W - 16 - text_w(&l2, 1);
        c.text(f, x1.max(260), 6, &fit(&l1, 1, W - 276), DIM, 1);
        c.text(f, x2.max(260), 28, &fit(&l2, 1, W - 276), DIM, 1);

        if let Some((ssid, text, pick, show)) = &password {
            draw_password(c, f, ssid, text, *pick, *show);
        } else {
            let first = sel.saturating_sub(ROWS - 1).min(items.len().saturating_sub(ROWS));
            for (i, it) in items.iter().enumerate().skip(first).take(ROWS) {
                let y = LIST_Y + (i - first) as i32 * ROW_H;
                let selected = i == sel;
                if selected {
                    c.rect(8, y, W - 16, ROW_H - 4, SEL);
                }
                let fg = match (it.enabled, selected) {
                    (true, _) => TEXT,
                    (false, true) => 0xb8c2cc,
                    (false, false) => OFF,
                };
                let vw = text_w(&it.value, 1).min(300);
                c.text_fit(f, 20, y + 1, &it.label, fg, 2, W - 60 - vw);
                let vc = if selected { TEXT } else { DIM };
                c.text_fit(f, W - 20 - vw, y + 11, &it.value, vc, 1, 300);
            }
            if items.len() > ROWS {
                // Scroll position.
                let track = STATUS_Y - LIST_Y - 8;
                let th = (track * ROWS as i32 / items.len() as i32).max(16);
                let ty = LIST_Y + (track - th) * first as i32 / (items.len() - ROWS) as i32;
                c.rect(W - 6, ty, 3, th, DIM);
            }
        }

        // Status line: a job, its outcome, an upload, or where the web page is.
        let (msg, col) = if let Some(b) = &s.busy {
            (format!("{spin} {b}…"), BUSY)
        } else if let Some(u) = &s.upload {
            let pct = (u.done * 100).checked_div(u.total).unwrap_or(0);
            (
                format!("{spin} Receiving {}: {pct}% of {} MB", u.name, u.total >> 20),
                BUSY,
            )
        } else if let Some((t, err, when)) = &s.note {
            if when.elapsed() < Duration::from_secs(8) {
                (t.clone(), if *err { ERR } else { OK })
            } else {
                (web_hint(&s), DIM)
            }
        } else {
            (web_hint(&s), DIM)
        };
        c.text_fit(f, 16, STATUS_Y + 8, &msg, col, 1, W - 32);

        // The keys under the screen.
        c.rect(0, FOOT_Y, W, H - FOOT_Y, BAR);
        for (i, l) in labels.iter().enumerate() {
            let slot = W / 5;
            let x = i as i32 * slot;
            if i > 0 {
                c.rect(x, FOOT_Y + 10, 1, H - FOOT_Y - 20, OFF);
            }
            let tw = text_w(l, 1);
            c.text(f, x + (slot - tw) / 2, FOOT_Y + 14, l, TEXT, 1);
        }
    }
}

fn centre(c: &mut Canvas, f: &Font, s: &str, y: i32, col: u32, scale: i32) {
    let s = fit(s, scale, W - 32);
    c.text(f, (W - text_w(&s, scale)) / 2, y, &s, col, scale);
}

fn header_status(s: &Shared) -> (String, String) {
    let ip = s.ips.first().map(|(_, ip)| ip.clone());
    let wifi = if !s.wifi.enabled {
        "Wi-Fi off".to_string()
    } else if let Some(c) = &s.wifi.connection {
        format!("Wi-Fi {c}")
    } else if s.wifi.device.is_none() {
        "No Wi-Fi adapter".to_string()
    } else {
        format!("Wi-Fi {}", s.wifi.state)
    };
    let l1 = match ip {
        Some(ip) => format!("{wifi}  {ip}"),
        None => wifi,
    };
    let connected = s.bt_paired.iter().filter(|d| d.connected).count();
    let l2 = if !s.bt.present {
        "Bluetooth: no adapter".to_string()
    } else if !s.bt.powered {
        "Bluetooth off".to_string()
    } else {
        format!("Bluetooth on, {connected} connected")
    };
    (l1, l2)
}

fn web_hint(s: &Shared) -> String {
    match s.ips.first() {
        Some((_, ip)) => format!("Game data and settings on the web: http://{ip}/  ({}.local)", s.hostname),
        None => "No network yet: set up Wi-Fi to reach the web page for game data.".to_string(),
    }
}

fn home_items(s: &Shared) -> Vec<Item> {
    let mut v = Vec::new();
    for g in &s.games {
        let (value, act) = if g.missing.is_empty() {
            ("ready".to_string(), Do::Play(g.id.clone()))
        } else {
            (format!("needs {} file{}", g.missing.len(), if g.missing.len() == 1 { "" } else { "s" }), Do::Play(g.id.clone()))
        };
        let mut it = item(format!("▶ {}", g.title), value, act);
        it.enabled = g.missing.is_empty();
        v.push(it);
    }
    let wifi = match (&s.wifi.connection, s.wifi.enabled) {
        (_, false) => "off".to_string(),
        (Some(c), _) => c.clone(),
        (None, _) => "not connected".to_string(),
    };
    v.push(item("Wi-Fi", wifi, Do::Wifi));
    let bt = if !s.bt.present {
        "no adapter".to_string()
    } else if !s.bt.powered {
        "off".to_string()
    } else {
        let c = s.bt_paired.iter().filter(|d| d.connected).count();
        format!("{} paired, {c} connected", s.bt_paired.len())
    };
    v.push(item("Bluetooth", bt, Do::Bt));
    v.push(item("Power off", "", Do::Power));
    v
}

fn wifi_items(s: &Shared) -> Vec<Item> {
    let mut v = vec![
        item("Wi-Fi", if s.wifi.enabled { "on" } else { "off" }, Do::WifiToggle(!s.wifi.enabled)),
        item("Scan for networks", "", Do::WifiScan),
    ];
    if s.wifi.device.is_none() {
        let mut it = item("No Wi-Fi adapter found", "", Do::Nothing(String::new()));
        it.enabled = false;
        v.push(it);
    }
    let mut seen: Vec<&str> = Vec::new();
    for n in &s.networks {
        let saved = s.saved.contains(&n.ssid);
        let in_use = n.in_use || s.wifi.connection.as_deref() == Some(n.ssid.as_str());
        let mut parts = vec![format!("{}%", n.signal)];
        parts.push(if n.security.is_empty() { "open".into() } else { n.security.clone() });
        if in_use {
            parts.insert(0, "connected".into());
        } else if saved {
            parts.insert(0, "saved".into());
        }
        let mut n2 = n.clone();
        n2.in_use = in_use;
        v.push(item(n.ssid.clone(), parts.join(" · "), Do::WifiPick(n2, saved)));
        seen.push(&n.ssid);
    }
    // Saved networks not in the last scan.
    for name in &s.saved {
        if seen.contains(&name.as_str()) {
            continue;
        }
        let in_use = s.wifi.connection.as_deref() == Some(name.as_str());
        let n = Network {
            ssid: name.clone(),
            signal: 0,
            security: "saved".into(),
            in_use,
        };
        v.push(item(name.clone(), if in_use { "connected" } else { "saved" }, Do::WifiPick(n, true)));
    }
    v
}

fn bt_items(s: &Shared) -> Vec<Item> {
    if !s.bt.present {
        let mut it = item("No Bluetooth adapter", "", Do::Nothing(String::new()));
        it.enabled = false;
        return vec![it];
    }
    let mut v = vec![item(
        "Bluetooth",
        if s.bt.powered { "on" } else { "off" },
        Do::BtToggle(!s.bt.powered),
    )];
    if !s.bt.powered {
        return v;
    }
    v.push(item("Scan for devices", "put the device in pairing mode", Do::BtScan));
    for d in &s.bt_paired {
        let state = if d.connected { "connected" } else { "paired" };
        v.push(item(
            d.name.clone(),
            format!("{state} · {}", net::kind_label(&d.kind)),
            Do::BtOpen(d.clone()),
        ));
    }
    for d in &s.bt_found {
        if s.bt_paired.iter().any(|p| p.mac == d.mac) {
            continue;
        }
        let rssi = d.rssi.map(|r| format!(" · {r} dBm")).unwrap_or_default();
        v.push(item(
            format!("+ {}", d.name),
            format!("pair · {}{rssi}", net::kind_label(&d.kind)),
            Do::BtPair(d.clone()),
        ));
    }
    v
}

fn wifi_scan(ctx: &Ctx) {
    ctx.spawn_job("Scanning for Wi-Fi networks", || net::wifi_scan(true), |c, nets| {
        let n = nets.len();
        lock(&c.shared).networks = nets;
        c.note(format!("{n} networks found"), false);
    });
}

fn bt_scan(ctx: &Ctx) {
    ctx.spawn_job("Looking for Bluetooth devices (12 s)", || net::bt_scan(12), |c, found| {
        let n = found.len();
        lock(&c.shared).bt_found = found;
        c.note(
            if n == 0 {
                "Nothing found: is the device in pairing mode?".to_string()
            } else {
                format!("{n} found: choose one to pair")
            },
            n == 0,
        );
    });
}

fn draw_password(c: &mut Canvas, f: &Font, ssid: &str, text: &str, pick: usize, show: bool) {
    c.text_fit(f, 20, LIST_Y + 4, &format!("Network: {ssid}"), DIM, 1, W - 40);
    // The field.
    let fy = LIST_Y + 30;
    c.frame(16, fy, W - 32, 56, 2, DIM);
    let shown: String = if show {
        text.to_string()
    } else {
        "•".repeat(text.chars().count())
    };
    let max = (W - 64) / 20;
    let tail: String = {
        let n = shown.chars().count();
        shown.chars().skip(n.saturating_sub(max as usize - 1)).collect()
    };
    let tw = c.text(f, 28, fy + 8, &tail, TEXT, 2);
    c.rect(28 + tw + 2, fy + 8, 3, 40, BUSY);

    // The character wheel: VFO turns, VOL/MFK push adds.
    let chars: Vec<char> = CHARSET.chars().collect();
    let n = chars.len() as i32;
    let wy = fy + 84;
    let cx = W / 2;
    for k in -7i32..=7 {
        let ch = chars[((pick as i32 + k).rem_euclid(n)) as usize];
        let label = if ch == ' ' { "␣".to_string() } else { ch.to_string() };
        if k == 0 {
            c.rect(cx - 24, wy - 6, 48, 72, SEL);
            c.text(f, cx - 15, wy, &label, TEXT, 3);
        } else {
            let x = cx + k * 46 - 10 + if k > 0 { 8 } else { -8 };
            c.text(f, x, wy + 12, &label, if k.abs() < 4 { TEXT } else { DIM }, 2);
        }
    }
    let hy = wy + 92;
    for (i, l) in [
        "VFO turns: choose · VOL or MFK push: add · MFK turns: abc / ABC / 123 / !?",
        "With a keyboard: type the password, Enter connects, Esc cancels.",
    ]
    .iter()
    .enumerate()
    {
        c.text_fit(f, 20, hy + i as i32 * 24, l, DIM, 1, W - 40);
    }
}

/// When the power key went down; held long enough, the radio switches off.
pub struct PowerKey {
    down: Option<Instant>,
}

pub const POWER_HOLD: Duration = Duration::from_millis(1500);

impl PowerKey {
    pub fn new() -> PowerKey {
        PowerKey { down: None }
    }
    pub fn press(&mut self) {
        self.down = Some(Instant::now());
    }
    /// Released: true for a short press.
    pub fn release(&mut self) -> bool {
        self.down.take().is_some_and(|t| t.elapsed() < POWER_HOLD)
    }
    /// Held long enough to switch off.
    pub fn held(&mut self) -> bool {
        if self.down.is_some_and(|t| t.elapsed() >= POWER_HOLD) {
            self.down = None;
            return true;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::games::Game;
    use crate::net::WifiStatus;

    fn ctx() -> Ctx {
        let (tx, _rx) = std::sync::mpsc::channel();
        std::mem::forget(_rx);
        let (ctx, _r) = Ctx::new(tx);
        std::mem::forget(_r);
        {
            let mut s = lock(&ctx.shared);
            s.hostname = "x6100".into();
            s.wifi = WifiStatus {
                enabled: true,
                device: Some("wlan0".into()),
                state: "connected".into(),
                connection: Some("HomeNet".into()),
            };
            s.ips = vec![("wlan0".into(), "192.0.2.10".into())];
            s.saved = vec!["HomeNet".into(), "Phone".into()];
            s.networks = vec![
                Network { ssid: "HomeNet".into(), signal: 72, security: "WPA2".into(), in_use: true },
                Network { ssid: "Kawiarnia Żółw".into(), signal: 55, security: "".into(), in_use: false },
                Network { ssid: "Neighbour 5G".into(), signal: 31, security: "WPA2 WPA3".into(), in_use: false },
            ];
            s.bt = net::BtStatus { present: true, powered: true, address: "00:11:22:33:44:55".into(), name: "X6100".into() };
            s.bt_paired = vec![BtDevice { mac: "AA:BB:CC:DD:EE:01".into(), name: "Magic Keyboard".into(), kind: "input".into(), connected: true, rssi: None }];
            s.bt_found = vec![BtDevice { mac: "AA:BB:CC:DD:EE:02".into(), name: "JBL Go 3".into(), kind: "audio".into(), connected: false, rssi: Some(-61) }];
            s.games = vec![
                Game { id: "baseq3".into(), title: "Quake III Arena".into(), missing: vec![] },
                Game { id: "missionpack".into(), title: "Quake III Team Arena".into(), missing: vec!["missionpack/pak1.pk3".into()] },
                Game { id: "threewave".into(), title: "threewave".into(), missing: vec![] },
            ];
        }
        ctx
    }

    /// `X6100_RENDER=dir cargo test render_screens -- --ignored`: raw
    /// 800x480 frames of each screen, for looking at them.
    #[test]
    #[ignore]
    fn render_screens() {
        let out = std::env::var("X6100_RENDER").expect("X6100_RENDER");
        let ctx = ctx();
        let mut ui = Ui::new();
        let shot = |ui: &mut Ui, name: &str| {
            ui.draw(&ctx);
            let bytes: Vec<u8> = ui.canvas.px.iter().flat_map(|p| p.to_le_bytes()).collect();
            std::fs::write(format!("{out}/{name}.raw"), bytes).unwrap();
        };
        shot(&mut ui, "home");
        ui.handle(Act::Down, &ctx);
        ui.handle(Act::Down, &ctx);
        ui.handle(Act::Down, &ctx);
        ctx.note("Connected to HomeNet", false);
        shot(&mut ui, "home-sel");
        ui.handle(Act::Soft(0), &ctx);
        ui.handle(Act::Turn(Knob::Vfo, 4), &ctx);
        lock(&ctx.shared).busy = Some("Scanning for Wi-Fi networks".into());
        shot(&mut ui, "wifi");
        lock(&ctx.shared).busy = None;
        ui.handle(Act::Push, &ctx);
        for c in "secret pw".chars() {
            ui.handle(Act::Char(c), &ctx);
        }
        ui.handle(Act::Turn(Knob::Vfo, 5), &ctx);
        shot(&mut ui, "password");
        ui.handle(Act::Back, &ctx);
        ui.handle(Act::Back, &ctx);
        ui.handle(Act::Soft(1), &ctx);
        ui.handle(Act::Down, &ctx);
        ui.handle(Act::Down, &ctx);
        shot(&mut ui, "bluetooth");
        ui.handle(Act::Back, &ctx);
        assert!(!ui.power_key());
        shot(&mut ui, "power");
        assert!(ui.power_key());
    }
}
