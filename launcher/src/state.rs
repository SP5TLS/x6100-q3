// SPDX-License-Identifier: MIT
//! State shared by the screen, the web page and the background threads.

use crate::games::Game;
use crate::input::Act;
use crate::net::{BtDevice, BtStatus, Network, WifiStatus};
use crate::sys::lock;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// To the main loop, which owns the screen and the game.
pub enum Msg {
    Input(Act),
    /// Start a game; the answer goes back on the channel.
    Play(String, Sender<Result<(), String>>),
    Stop,
    PowerOff,
    /// Something in the shared state changed: draw again.
    Wake,
}

#[derive(Clone, Debug)]
pub struct Upload {
    pub name: String,
    pub done: u64,
    pub total: u64,
}

#[derive(Default, Clone)]
pub struct Shared {
    pub wifi: WifiStatus,
    pub ips: Vec<(String, String)>,
    pub saved: Vec<String>,
    /// Last Wi-Fi scan.
    pub networks: Vec<Network>,
    pub bt: BtStatus,
    pub bt_paired: Vec<BtDevice>,
    /// Last Bluetooth scan (devices not paired yet).
    pub bt_found: Vec<BtDevice>,
    pub games: Vec<Game>,
    /// The game running and since when.
    pub playing: Option<(String, Instant)>,
    /// What a background job is doing ("Scanning for networks").
    pub busy: Option<String>,
    /// The last job's outcome: text, error?, when.
    pub note: Option<(String, bool, Instant)>,
    pub upload: Option<Upload>,
    pub free_home: Option<u64>,
    pub powering_off: bool,
    pub hostname: String,
}

#[derive(Clone)]
pub struct Ctx {
    pub shared: Arc<Mutex<Shared>>,
    pub tx: Sender<Msg>,
    refresh: Sender<()>,
}

impl Ctx {
    pub fn new(tx: Sender<Msg>) -> (Ctx, Receiver<()>) {
        let (refresh, rx) = std::sync::mpsc::channel();
        let shared = Shared {
            hostname: crate::sys::hostname(),
            ..Shared::default()
        };
        (
            Ctx {
                shared: Arc::new(Mutex::new(shared)),
                tx,
                refresh,
            },
            rx,
        )
    }

    pub fn wake(&self) {
        let _ = self.tx.send(Msg::Wake);
    }

    /// Have the poller look at everything again now.
    pub fn refresh(&self) {
        let _ = self.refresh.send(());
    }

    pub fn note(&self, text: impl Into<String>, error: bool) {
        lock(&self.shared).note = Some((text.into(), error, Instant::now()));
        self.wake();
    }

    /// Run a slow job (nmcli, x6100-bt) in this thread, one at a time;
    /// the screen shows `what` meanwhile and the outcome afterwards.
    pub fn job<T>(&self, what: &str, f: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
        {
            let mut s = lock(&self.shared);
            if let Some(b) = &s.busy {
                return Err(format!("Busy: {b}"));
            }
            s.busy = Some(what.to_string());
        }
        self.wake();
        let r = f();
        lock(&self.shared).busy = None;
        if let Err(e) = &r {
            self.note(format!("{what}: {e}"), true);
        }
        self.refresh();
        self.wake();
        r
    }

    /// The same in a thread of its own; `done` reports success.
    pub fn spawn_job<T: Send + 'static>(
        &self,
        what: &str,
        f: impl FnOnce() -> Result<T, String> + Send + 'static,
        done: impl FnOnce(&Ctx, T) + Send + 'static,
    ) {
        let ctx = self.clone();
        let what = what.to_string();
        std::thread::spawn(move || {
            if let Ok(v) = ctx.job(&what, f) {
                done(&ctx, v);
            }
        });
    }
}

/// Background thread: Wi-Fi, addresses, Bluetooth and game data, every
/// few seconds (less often while the game runs) and on request.
pub fn start_poller(ctx: Ctx, rx: Receiver<()>) {
    std::thread::Builder::new()
        .name("poller".into())
        .spawn(move || loop {
            let playing = lock(&ctx.shared).playing.is_some();
            let wifi = crate::net::wifi_status();
            let ips = crate::sys::ipv4_addresses();
            let saved = crate::net::wifi_saved();
            let bt = crate::net::bt_status();
            let paired = if bt.present { crate::net::bt_devices() } else { Vec::new() };
            let games = crate::games::scan();
            let free = crate::sys::free_bytes(&crate::games::home_dir());
            {
                let mut s = lock(&ctx.shared);
                s.wifi = wifi;
                s.ips = ips;
                s.saved = saved;
                s.bt = bt;
                s.bt_paired = paired;
                s.games = games;
                s.free_home = free;
            }
            ctx.wake();
            let wait = if playing { 20 } else { 5 };
            let _ = rx.recv_timeout(Duration::from_secs(wait));
            // Several requests at once: one look is enough.
            while rx.try_recv().is_ok() {}
        })
        .expect("poller thread");
}
