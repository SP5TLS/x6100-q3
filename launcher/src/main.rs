// SPDX-License-Identifier: MIT
//! x6100-launcher: what the X6100 shows when it starts (x6100-q3).
//!
//! - On the screen: play Quake 3, set up Wi-Fi and Bluetooth, power off,
//!   with the radio's knobs and keys or a keyboard and mouse.
//! - On the web (port 80): upload the game data, the same settings,
//!   start and stop the game.
//! - The game runs as a child; when it ends the menu comes back. The
//!   power key stops the game (short press) or switches off (held).
//!
//!   x6100-launcher [--fb DEVICE] [--web ADDR:PORT] [--no-input]
//!   x6100-launcher --games      list games and the files they still need

mod font;
mod game;
mod games;
mod gfx;
mod input;
mod net;
mod state;
mod sys;
mod ui;
mod web;

use state::{Ctx, Msg};
use std::sync::mpsc::RecvTimeoutError;
use std::time::{Duration, Instant};
use sys::lock;

const USAGE: &str = "usage: x6100-launcher [--fb DEVICE] [--web ADDR:PORT] [--no-input]
       x6100-launcher --games";

struct App {
    ctx: Ctx,
    ui: ui::Ui,
    fb: Option<gfx::Fb>,
    fb_path: String,
    runner: game::Runner,
    power: ui::PowerKey,
}

impl App {
    fn present(&mut self) {
        self.ui.draw(&self.ctx);
        if let Some(fb) = self.fb.as_mut() {
            if let Err(e) = fb.present(&self.ui.canvas) {
                eprintln!("launcher: {}: {e}", self.fb_path);
                self.fb = None;
            }
        }
    }

    fn play(&mut self, g: &str) -> Result<(), String> {
        if !games::valid_name(g) {
            return Err("no such game".into());
        }
        if self.runner.running() {
            return Err("a game is running already".into());
        }
        let missing = games::missing(g);
        if !missing.is_empty() {
            return Err(format!("{} needs {}", games::title(g), missing.join(" ")));
        }
        lock(&self.ctx.shared).playing = Some((g.to_string(), Instant::now()));
        self.present();
        if let Err(e) = self.runner.start(g) {
            lock(&self.ctx.shared).playing = None;
            self.ctx.note(e.clone(), true);
            return Err(e);
        }
        self.ctx.refresh();
        Ok(())
    }

    fn game_ended(&mut self, end: game::Ended) {
        lock(&self.ctx.shared).playing = None;
        if end.code != Some(0) && end.ran < Duration::from_secs(20) {
            let why = game::log_hint().unwrap_or_else(|| "see /data/log/quake3.log".into());
            self.ctx.note(format!("{} stopped: {why}", games::title(&end.game)), true);
        }
        // The game had the display: take it back as it is now.
        sys::console_graphics();
        if let Some(fb) = self.fb.as_mut() {
            fb.refresh_geometry(&self.fb_path);
        }
        self.ui.home();
        self.ctx.refresh();
    }

    fn power_off(&mut self) {
        lock(&self.ctx.shared).powering_off = true;
        self.present();
        if self.runner.running() {
            self.runner.stop();
            let t = Instant::now();
            while self.runner.running() && t.elapsed() < Duration::from_secs(7) {
                self.runner.poll();
                std::thread::sleep(Duration::from_millis(100));
            }
        }
        eprintln!("launcher: switching off");
        let r = std::process::Command::new(sys::path("/usr/bin/x6100-poweroff")).status();
        if let Err(e) = r {
            eprintln!("launcher: x6100-poweroff: {e}");
            lock(&self.ctx.shared).powering_off = false;
        }
    }

    fn input(&mut self, a: input::Act) {
        use input::Act;
        match a {
            Act::PowerDown => self.power.press(),
            Act::PowerUp => {
                if self.power.release() {
                    if self.runner.running() {
                        self.runner.stop();
                    } else if self.ui.power_key() {
                        self.power_off();
                    }
                }
            }
            // While the game runs, the game has the keyboard and mouse.
            _ if self.runner.running() => {}
            a => match self.ui.handle(a, &self.ctx) {
                ui::Out::Play(g) => {
                    let _ = self.play(&g);
                }
                ui::Out::PowerOff => self.power_off(),
                ui::Out::None => {}
            },
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut fb_path = "/dev/fb0".to_string();
    let mut web_addr = None;
    let mut with_input = true;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--games" => {
                games::print_list();
                return;
            }
            "--fb" => fb_path = it.next().cloned().unwrap_or_default(),
            "--web" => web_addr = it.next().cloned(),
            "--no-input" => with_input = false,
            "--version" => {
                println!("x6100-launcher {}", env!("CARGO_PKG_VERSION"));
                return;
            }
            _ => {
                eprintln!("{USAGE}");
                std::process::exit(2);
            }
        }
    }

    let (tx, rx) = std::sync::mpsc::channel::<Msg>();
    let (ctx, refresh_rx) = Ctx::new(tx.clone());
    state::start_poller(ctx.clone(), refresh_rx);
    if with_input {
        input::start(tx.clone(), Msg::Input);
    }
    let addr = web_addr.unwrap_or_else(|| {
        if sys::cfg_on("WEB", "on") { "0.0.0.0:80" } else { "127.0.0.1:80" }.to_string()
    });
    if let Err(e) = web::start(ctx.clone(), &addr) {
        eprintln!("launcher: web page on {addr}: {e}");
    }

    sys::console_graphics();
    let fb = match gfx::Fb::open(&fb_path, 270) {
        Ok(fb) => Some(fb),
        Err(e) => {
            eprintln!("launcher: {fb_path}: {e} (no screen)");
            None
        }
    };
    let mut app = App {
        ctx: ctx.clone(),
        ui: ui::Ui::new(),
        fb,
        fb_path,
        runner: game::Runner::new(),
        power: ui::PowerKey::new(),
    };
    eprintln!("launcher: x6100-launcher {} started", env!("CARGO_PKG_VERSION"));

    // QUAKE3=boot: the game at once on the first start after power-on.
    let marker = sys::path("/run/x6100/launcher-started");
    let first = !marker.exists();
    if let Some(d) = marker.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let _ = std::fs::write(&marker, b"");
    if first && sys::cfg("QUAKE3", "off") == "boot" {
        let g = sys::cfg("QUAKE3_GAME", "baseq3");
        if let Err(e) = app.play(&g) {
            app.ctx.note(format!("QUAKE3=boot: {e}"), true);
        }
    }

    let mut dirty = true;
    let mut drawn = Instant::now() - Duration::from_secs(10);
    let mut fb_retry = Instant::now();
    loop {
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(m) => {
                let mut next = Some(m);
                // Everything that is waiting, then one frame.
                while let Some(m) = next {
                    match m {
                        Msg::Input(a) => app.input(a),
                        Msg::Play(g, reply) => {
                            let _ = reply.send(app.play(&g));
                        }
                        Msg::Stop => app.runner.stop(),
                        Msg::PowerOff => app.power_off(),
                        Msg::Wake => {}
                    }
                    dirty = true;
                    next = rx.try_recv().ok();
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
        if app.power.held() {
            app.power_off();
        }
        if let Some(end) = app.runner.poll() {
            app.game_ended(end);
            dirty = true;
        }
        if app.runner.running() {
            continue;
        }
        // The panel may come up after the launcher: keep trying.
        if app.fb.is_none() && fb_retry.elapsed() >= Duration::from_secs(3) {
            fb_retry = Instant::now();
            if let Ok(fb) = gfx::Fb::open(&app.fb_path, 270) {
                eprintln!("launcher: {} is there now", app.fb_path);
                sys::console_graphics();
                app.fb = Some(fb);
                dirty = true;
            }
        }
        let animating = ui::Ui::animating(&lock(&app.ctx.shared));
        let due = if animating { 250 } else { 1000 };
        if dirty || drawn.elapsed() >= Duration::from_millis(due) {
            app.present();
            drawn = Instant::now();
            dirty = false;
        }
    }
}
