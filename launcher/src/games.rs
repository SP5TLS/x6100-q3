// SPDX-License-Identifier: MIT
//! Quake 3 game data: which games are there and which files they still
//! need. Two places, as ioquake3's base and home path: /data/quake3
//! (preloaded into private images) and /mnt/quake3 (the X6100 partition,
//! where uploads go).

use crate::sys::path;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq)]
pub struct Game {
    pub id: String,
    pub title: String,
    /// Files still needed, e.g. "baseq3/pak0.pk3"; empty when playable.
    pub missing: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct DataFile {
    pub game: String,
    pub name: String,
    pub size: u64,
}

pub fn base_dir() -> PathBuf {
    path("/data/quake3")
}

/// Where uploads go.
pub fn home_dir() -> PathBuf {
    path("/mnt/quake3")
}

fn roots() -> [PathBuf; 2] {
    [base_dir(), home_dir()]
}

pub fn title(id: &str) -> String {
    match id {
        "baseq3" => "Quake III Arena".into(),
        "missionpack" => "Quake III Team Arena".into(),
        "baseoa" => "OpenArena".into(),
        _ => id.into(),
    }
}

/// A game directory or file name we accept from the network: letters,
/// digits, '.', '_', '-', not starting with '.', at most 64 characters.
pub fn valid_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && !s.starts_with('.')
        && s.bytes().all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
}

/// Files in `root/game/` (lower-case names; only regular files).
fn files_in(root: &Path, game: &str) -> Vec<(String, u64)> {
    let Ok(dir) = std::fs::read_dir(root.join(game)) else {
        return Vec::new();
    };
    let mut v: Vec<(String, u64)> = dir
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let md = e.metadata().ok()?;
            md.is_file()
                .then(|| (e.file_name().to_string_lossy().to_string(), md.len()))
        })
        .filter(|(n, _)| !n.starts_with('.'))
        .collect();
    v.sort();
    v
}

fn has_pk3(root: &Path, game: &str) -> bool {
    files_in(root, game)
        .iter()
        .any(|(n, _)| n.to_ascii_lowercase().ends_with(".pk3"))
}

fn has(game: &str, file: &str) -> bool {
    roots().iter().any(|r| {
        files_in(r, game)
            .iter()
            .any(|(n, _)| n.eq_ignore_ascii_case(file))
    })
}

pub fn missing(game: &str) -> Vec<String> {
    let mut m = Vec::new();
    if game == "baseoa" {
        if !has("baseoa", "pak0.pk3") {
            m.push("baseoa/pak0.pk3".to_string());
        }
        return m;
    }
    for i in 0..=8 {
        let f = format!("pak{i}.pk3");
        if !has("baseq3", &f) {
            m.push(format!("baseq3/{f}"));
        }
    }
    if game == "missionpack" {
        for i in 0..=3 {
            let f = format!("pak{i}.pk3");
            if !has("missionpack", &f) {
                m.push(format!("missionpack/{f}"));
            }
        }
    } else if game != "baseq3" && !roots().iter().any(|r| has_pk3(r, game)) {
        m.push(format!("{game}/*.pk3"));
    }
    m
}

/// Games found (directories with .pk3 files), Quake III Arena first and
/// always listed.
pub fn scan() -> Vec<Game> {
    let mut ids: Vec<String> = Vec::new();
    for r in roots() {
        let Ok(dir) = std::fs::read_dir(&r) else { continue };
        for e in dir.filter_map(|e| e.ok()) {
            let id = e.file_name().to_string_lossy().to_string();
            if valid_name(&id) && e.path().is_dir() && has_pk3(&r, &id) && !ids.contains(&id) {
                ids.push(id);
            }
        }
    }
    let rank = |id: &str| match id {
        "baseq3" => 0,
        "missionpack" => 1,
        "baseoa" => 2,
        _ => 3,
    };
    if !ids.iter().any(|i| i == "baseq3") {
        ids.push("baseq3".into());
    }
    ids.sort_by(|a, b| rank(a).cmp(&rank(b)).then(a.cmp(b)));
    ids.into_iter()
        .map(|id| Game {
            title: title(&id),
            missing: missing(&id),
            id,
        })
        .collect()
}

/// Files for the web page: (root path, writable, files).
pub fn listing() -> Vec<(PathBuf, bool, Vec<DataFile>)> {
    roots()
        .into_iter()
        .enumerate()
        .map(|(i, r)| {
            let mut files = Vec::new();
            if let Ok(dir) = std::fs::read_dir(&r) {
                let mut games: Vec<String> = dir
                    .filter_map(|e| e.ok())
                    .filter(|e| e.path().is_dir())
                    .map(|e| e.file_name().to_string_lossy().to_string())
                    .filter(|g| valid_name(g))
                    .collect();
                games.sort();
                for g in games {
                    for (name, size) in files_in(&r, &g) {
                        files.push(DataFile {
                            game: g.clone(),
                            name,
                            size,
                        });
                    }
                }
            }
            // Uploads and deletions only on the X6100 partition.
            (r, i == 1, files)
        })
        .collect()
}

/// `x6100-launcher --games` (and `quake3 list`).
pub fn print_list() {
    for g in scan() {
        if g.missing.is_empty() {
            println!("{:<12} {:<22} ready", g.id, g.title);
        } else {
            println!("{:<12} {:<22} missing: {}", g.id, g.title, g.missing.join(" "));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert!(valid_name("pak0.pk3"));
        assert!(valid_name("baseq3"));
        assert!(valid_name("q3wpak0-4.pk3"));
        assert!(!valid_name(".."));
        assert!(!valid_name(".hidden"));
        assert!(!valid_name("a/b"));
        assert!(!valid_name("a b"));
        assert!(!valid_name(""));
        assert!(!valid_name(&"x".repeat(65)));
    }
}
