// SPDX-License-Identifier: MIT
//! The game as a child process (/usr/libexec/x6100/quake3-run GAME) in a
//! process group of its own, so Stop reaches ioquake3 too.

use crate::sys::{kill_group, SIGKILL, SIGTERM};
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

pub struct Runner {
    child: Option<Child>,
    game: String,
    started: Instant,
    kill_at: Option<Instant>,
}

pub struct Ended {
    pub game: String,
    pub code: Option<i32>,
    pub ran: Duration,
}

impl Runner {
    pub fn new() -> Runner {
        Runner {
            child: None,
            game: String::new(),
            started: Instant::now(),
            kill_at: None,
        }
    }

    pub fn running(&self) -> bool {
        self.child.is_some()
    }

    pub fn start(&mut self, game: &str) -> Result<(), String> {
        if self.child.is_some() {
            return Err(format!("{} is running already", crate::games::title(&self.game)));
        }
        let run = crate::sys::path("/usr/libexec/x6100/quake3-run");
        let child = Command::new(&run)
            .arg(game)
            .process_group(0)
            .stdin(Stdio::null())
            .spawn()
            .map_err(|e| format!("{}: {e}", run.display()))?;
        eprintln!("launcher: started {game} (pid {})", child.id());
        self.child = Some(child);
        self.game = game.to_string();
        self.started = Instant::now();
        self.kill_at = None;
        Ok(())
    }

    /// SIGTERM to the game; SIGKILL 5 s later if it is still there
    /// (ioquake3 can hang in the GL driver when it shuts down on a signal).
    pub fn stop(&mut self) {
        if let Some(c) = &self.child {
            if self.kill_at.is_none() {
                eprintln!("launcher: stopping {}", self.game);
                kill_group(c.id(), SIGTERM);
                self.kill_at = Some(Instant::now() + Duration::from_secs(5));
            }
        }
    }

    /// Call often: Some once the game has ended.
    pub fn poll(&mut self) -> Option<Ended> {
        let c = self.child.as_mut()?;
        match c.try_wait() {
            Ok(Some(st)) => {
                let pid = c.id();
                // Whatever is left of the group (it should be nothing).
                kill_group(pid, SIGKILL);
                self.child = None;
                eprintln!("launcher: {} ended ({st})", self.game);
                Some(Ended {
                    game: std::mem::take(&mut self.game),
                    code: st.code(),
                    ran: self.started.elapsed(),
                })
            }
            Ok(None) => {
                if self.kill_at.is_some_and(|t| Instant::now() >= t) {
                    eprintln!("launcher: {} did not stop, killing it", self.game);
                    kill_group(c.id(), SIGKILL);
                    self.kill_at = Some(Instant::now() + Duration::from_secs(60));
                }
                None
            }
            Err(_) => None,
        }
    }
}

/// The last lines of the game's log that say what went wrong.
pub fn log_hint() -> Option<String> {
    let log = std::fs::read_to_string(crate::sys::path("/data/log/quake3.log")).ok()?;
    log.lines()
        .rev()
        .map(str::trim)
        .find(|l| {
            let lc = l.to_ascii_lowercase();
            lc.contains("error") || lc.contains("couldn't") || lc.contains("failed")
        })
        .map(|l| l.chars().take(120).collect())
}
