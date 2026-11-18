// SPDX-License-Identifier: MIT
//! Wi-Fi through NetworkManager (`nmcli`) and Bluetooth through `x6100-bt`
//! (BlueZ's bluetoothctl plus the x6100-btd pairing agent).

use crate::sys::{run, split_terse};
use std::time::Duration;

const T: Duration = Duration::from_secs(10);

#[derive(Clone, Debug, Default)]
pub struct WifiStatus {
    pub enabled: bool,
    /// The Wi-Fi device (wlan0), if there is one.
    pub device: Option<String>,
    /// NetworkManager's state of it: connected, disconnected, ...
    pub state: String,
    /// The active connection (its name: the network's SSID).
    pub connection: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Network {
    pub ssid: String,
    pub signal: u8,
    pub security: String,
    pub in_use: bool,
}

pub fn wifi_status() -> WifiStatus {
    let mut st = WifiStatus::default();
    if let Ok(out) = run("nmcli", &["-t", "-f", "WIFI", "radio"], T) {
        st.enabled = out.trim() == "enabled";
    }
    if let Ok(out) = run("nmcli", &["-t", "-f", "DEVICE,TYPE,STATE,CONNECTION", "device"], T) {
        for line in out.lines() {
            let f = split_terse(line);
            if f.len() >= 4 && f[1] == "wifi" {
                st.device = Some(f[0].clone());
                st.state = f[2].clone();
                st.connection = (!f[3].is_empty() && f[2].starts_with("connected"))
                    .then(|| f[3].clone());
                break;
            }
        }
    }
    st
}

/// Networks in range, strongest first, one entry per name.
pub fn wifi_scan(rescan: bool) -> Result<Vec<Network>, String> {
    let out = run(
        "nmcli",
        &[
            "-t",
            "-f",
            "IN-USE,SSID,SIGNAL,SECURITY",
            "device",
            "wifi",
            "list",
            "--rescan",
            if rescan { "yes" } else { "no" },
        ],
        Duration::from_secs(30),
    )?;
    Ok(parse_scan(&out))
}

fn parse_scan(out: &str) -> Vec<Network> {
    let mut v: Vec<Network> = Vec::new();
    for line in out.lines() {
        let f = split_terse(line);
        if f.len() < 4 || f[1].is_empty() {
            continue;
        }
        let n = Network {
            in_use: f[0].trim() == "*",
            ssid: f[1].clone(),
            signal: f[2].parse().unwrap_or(0),
            security: if f[3] == "--" { String::new() } else { f[3].clone() },
        };
        match v.iter_mut().find(|x| x.ssid == n.ssid) {
            Some(x) => {
                x.in_use |= n.in_use;
                if n.signal > x.signal {
                    x.signal = n.signal;
                    x.security = n.security;
                }
            }
            None => v.push(n),
        }
    }
    v.sort_by(|a, b| b.in_use.cmp(&a.in_use).then(b.signal.cmp(&a.signal)));
    v
}

/// Saved Wi-Fi connections (their names; nmcli names them by SSID).
pub fn wifi_saved() -> Vec<String> {
    let Ok(out) = run("nmcli", &["-t", "-f", "NAME,TYPE", "connection", "show"], T) else {
        return Vec::new();
    };
    out.lines()
        .map(split_terse)
        .filter(|f| f.len() >= 2 && f[1] == "802-11-wireless")
        .map(|f| f[0].clone())
        .collect()
}

/// Connect: a saved network by name, or a new one with its password
/// (None for an open network).
pub fn wifi_connect(ssid: &str, password: Option<&str>, saved: bool) -> Result<String, String> {
    let long = Duration::from_secs(60);
    if saved && password.is_none() {
        run("nmcli", &["--wait", "45", "connection", "up", "id", ssid], long)?;
    } else {
        // A saved network with a new password: start again from scratch.
        if saved {
            let _ = run("nmcli", &["connection", "delete", "id", ssid], T);
        }
        let mut args = vec!["--wait", "45", "device", "wifi", "connect", ssid];
        if let Some(p) = password {
            args.extend(["password", p]);
        }
        run("nmcli", &args, long)?;
    }
    Ok(format!("Connected to {ssid}"))
}

pub fn wifi_forget(name: &str) -> Result<String, String> {
    run("nmcli", &["connection", "delete", "id", name], T)?;
    Ok(format!("Forgot {name}"))
}

pub fn wifi_enable(on: bool) -> Result<String, String> {
    run("nmcli", &["radio", "wifi", if on { "on" } else { "off" }], T)?;
    Ok(format!("Wi-Fi {}", if on { "on" } else { "off" }))
}

#[derive(Clone, Debug, Default)]
pub struct BtStatus {
    /// An adapter answers.
    pub present: bool,
    pub powered: bool,
    pub address: String,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BtDevice {
    pub mac: String,
    pub name: String,
    /// audio, headset, input, phone, computer or other.
    pub kind: String,
    pub connected: bool,
    /// Signal strength from a scan (dBm), if known.
    pub rssi: Option<i32>,
}

pub fn bt_status() -> BtStatus {
    match run("x6100-bt", &["status"], T) {
        Ok(out) => {
            let f = split_terse(out.lines().next().unwrap_or(""));
            if f.len() < 5 {
                return BtStatus::default();
            }
            BtStatus {
                present: true,
                powered: f[0] == "1",
                address: f[3].clone(),
                name: f[4].clone(),
            }
        }
        Err(_) => BtStatus::default(),
    }
}

/// Paired devices.
pub fn bt_devices() -> Vec<BtDevice> {
    let Ok(out) = run("x6100-bt", &["devices"], T) else {
        return Vec::new();
    };
    out.lines()
        .map(split_terse)
        .filter(|f| f.len() >= 5)
        .map(|f| BtDevice {
            mac: f[0].clone(),
            connected: f[1] == "1",
            kind: f[3].clone(),
            name: f[4].clone(),
            rssi: None,
        })
        .collect()
}

/// Devices in range that are not paired yet (takes `secs`).
pub fn bt_scan(secs: u32) -> Result<Vec<BtDevice>, String> {
    let s = secs.to_string();
    let out = run("x6100-bt", &["scan", &s], Duration::from_secs(secs as u64 + 20))?;
    Ok(out
        .lines()
        .map(split_terse)
        .filter(|f| f.len() >= 4)
        .map(|f| BtDevice {
            mac: f[0].clone(),
            rssi: f[1].parse().ok(),
            kind: f[2].clone(),
            name: f[3].clone(),
            connected: false,
        })
        .collect())
}

pub fn valid_mac(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 17
        && b.iter().enumerate().all(|(i, c)| {
            if i % 3 == 2 {
                *c == b':'
            } else {
                c.is_ascii_hexdigit()
            }
        })
}

/// pair, connect, disconnect, forget (with a device address), on, off.
pub fn bt(cmd: &str, mac: Option<&str>) -> Result<String, String> {
    let mut args = vec![cmd];
    if let Some(m) = mac {
        if !valid_mac(m) {
            return Err("not a Bluetooth address".into());
        }
        args.push(m);
    }
    let t = if cmd == "pair" { 90 } else { 40 };
    run("x6100-bt", &args, Duration::from_secs(t))?;
    Ok(match cmd {
        "pair" => "Paired".to_string(),
        "connect" => "Connected".to_string(),
        "disconnect" => "Disconnected".to_string(),
        "forget" => "Forgotten".to_string(),
        "on" => "Bluetooth on".to_string(),
        "off" => "Bluetooth off".to_string(),
        _ => "Done".to_string(),
    })
}

pub fn kind_label(kind: &str) -> &'static str {
    match kind {
        "audio" => "audio",
        "headset" => "headset",
        "input" => "keyboard/mouse",
        "phone" => "phone",
        "computer" => "computer",
        _ => "device",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_list() {
        let out = "*:Home\\:Net:72:WPA2\n :Home\\:Net:40:WPA2\n :Cafe:80:--\n ::90:WPA2\n";
        let v = parse_scan(out);
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].ssid, "Home:Net");
        assert!(v[0].in_use);
        assert_eq!(v[0].signal, 72);
        assert_eq!(v[1].ssid, "Cafe");
        assert_eq!(v[1].security, "");
    }

    #[test]
    fn macs() {
        assert!(valid_mac("00:11:22:AA:bb:FF"));
        assert!(!valid_mac("00:11:22:AA:bb"));
        assert!(!valid_mac("00:11:22:AA:bb:FG"));
        assert!(!valid_mac("00-11-22-AA-bb-FF"));
    }
}
