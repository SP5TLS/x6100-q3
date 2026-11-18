// SPDX-License-Identifier: MIT
//! The web page and its API on port 80: game data upload, Wi-Fi,
//! Bluetooth, start/stop the game, power off, licences.
//!
//! Plain HTTP/1.1, one thread per connection, `Connection: close`.
//! Requests that change something need the header `X-Requested-With:
//! x6100` (a page from another site cannot send it without CORS, which is
//! never granted), and the Host header must name this radio (an address,
//! its name or name.local), which keeps DNS rebinding out. WEB_PASSWORD in
//! x6100.conf adds HTTP Basic authentication (not for this machine).

use crate::state::{Ctx, Msg, Upload};
use crate::sys::{lock, path};
use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

static INDEX: &str = include_str!("index.html");
static LICENSE: &str = include_str!("../LICENSE");
static FONT_NOTICE: &str = include_str!("../fonts/README.md");

const MAX_CONNECTIONS: usize = 24;
/// FAT32's largest file.
const MAX_UPLOAD: u64 = (4 << 30) - 1;

struct Server {
    ctx: Ctx,
    password: String,
    active: AtomicUsize,
}

pub fn start(ctx: Ctx, addr: &str) -> io::Result<()> {
    let listener = TcpListener::bind(addr)?;
    eprintln!("launcher: web page on http://{addr}/");
    let srv = Arc::new(Server {
        ctx,
        password: crate::sys::cfg("WEB_PASSWORD", ""),
        active: AtomicUsize::new(0),
    });
    std::thread::Builder::new()
        .name("web".into())
        .spawn(move || {
            for conn in listener.incoming() {
                let Ok(stream) = conn else { continue };
                if srv.active.load(Ordering::Relaxed) >= MAX_CONNECTIONS {
                    let mut s = stream;
                    let _ = respond(&mut s, 503, "text/plain", b"Busy, try again\n", &[]);
                    continue;
                }
                srv.active.fetch_add(1, Ordering::Relaxed);
                let srv = srv.clone();
                let _ = std::thread::Builder::new().name("web-conn".into()).spawn(move || {
                    let _ = connection(&srv, stream);
                    srv.active.fetch_sub(1, Ordering::Relaxed);
                });
            }
        })?;
    Ok(())
}

struct Req {
    method: String,
    path: String,
    query: Vec<(String, String)>,
    headers: Vec<(String, String)>,
    peer: SocketAddr,
}

impl Req {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
    fn q(&self, key: &str) -> &str {
        self.query
            .iter()
            .find(|(k, _)| k == key)
            .map_or("", |(_, v)| v.as_str())
    }
    fn local(&self) -> bool {
        self.peer.ip().is_loopback()
    }
}

/// The request body: what was read with the headers, then the socket.
struct Body<'a> {
    pre: Vec<u8>,
    pos: usize,
    stream: &'a mut TcpStream,
    left: u64,
}

impl Read for Body<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.left == 0 {
            return Ok(0);
        }
        let max = buf.len().min(self.left.min(usize::MAX as u64) as usize);
        let n = if self.pos < self.pre.len() {
            let n = max.min(self.pre.len() - self.pos);
            buf[..n].copy_from_slice(&self.pre[self.pos..self.pos + n]);
            self.pos += n;
            n
        } else {
            let n = self.stream.read(&mut buf[..max])?;
            if n == 0 {
                return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "connection closed"));
            }
            n
        };
        self.left -= n as u64;
        Ok(n)
    }
}

fn connection(srv: &Server, mut stream: TcpStream) -> io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(60)))?;
    stream.set_write_timeout(Some(Duration::from_secs(60)))?;
    let peer = stream.peer_addr()?;

    // Headers, at most 16 KiB.
    let mut buf = Vec::with_capacity(4096);
    let mut chunk = [0u8; 4096];
    let end = loop {
        if let Some(i) = find(&buf, b"\r\n\r\n") {
            break i;
        }
        if buf.len() > 16384 {
            return respond(&mut stream, 431, "text/plain", b"Headers too large\n", &[]);
        }
        let n = stream.read(&mut chunk)?;
        if n == 0 {
            return Ok(());
        }
        buf.extend_from_slice(&chunk[..n]);
    };
    let head = String::from_utf8_lossy(&buf[..end]).to_string();
    let pre = buf[end + 4..].to_vec();
    let mut lines = head.split("\r\n");
    let mut first = lines.next().unwrap_or("").split(' ');
    let method = first.next().unwrap_or("").to_string();
    let target = first.next().unwrap_or("/").to_string();
    let headers: Vec<(String, String)> = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        .collect();
    let (p, q) = target.split_once('?').unwrap_or((&target, ""));
    let req = Req {
        method,
        path: url_decode(p),
        query: parse_form(q),
        headers,
        peer,
    };

    if !host_ok(&req, &srv.ctx) {
        return respond(&mut stream, 403, "text/plain", b"Use the radio's address or name\n", &[]);
    }
    if !srv.password.is_empty() && !req.local() && !auth_ok(&req, &srv.password) {
        return respond(
            &mut stream,
            401,
            "text/plain",
            b"Password needed (WEB_PASSWORD in x6100.conf)\n",
            &[("WWW-Authenticate", "Basic realm=\"x6100\"")],
        );
    }
    let changes = !matches!(req.method.as_str(), "GET" | "HEAD");
    if changes && req.header("X-Requested-With") != Some("x6100") {
        return respond(&mut stream, 403, "text/plain", b"Missing X-Requested-With\n", &[]);
    }
    if req.header("Transfer-Encoding").is_some() {
        return respond(&mut stream, 411, "text/plain", b"Send a Content-Length\n", &[]);
    }
    let len: u64 = req.header("Content-Length").and_then(|v| v.parse().ok()).unwrap_or(0);
    if req.header("Expect").is_some_and(|v| v.eq_ignore_ascii_case("100-continue")) {
        stream.write_all(b"HTTP/1.1 100 Continue\r\n\r\n")?;
    }
    let mut body = Body {
        pre,
        pos: 0,
        stream: &mut stream,
        left: len,
    };
    let r = route(srv, &req, &mut body, len);
    respond(&mut stream, r.status, r.ctype, &r.body, &[])
}

struct Resp {
    status: u16,
    ctype: &'static str,
    body: Vec<u8>,
}

fn json(status: u16, body: String) -> Resp {
    Resp {
        status,
        ctype: "application/json",
        body: body.into_bytes(),
    }
}

fn ok(msg: &str) -> Resp {
    json(200, format!("{{\"ok\":true,\"message\":{}}}", js(msg)))
}

fn err(status: u16, msg: &str) -> Resp {
    json(status, format!("{{\"ok\":false,\"message\":{}}}", js(msg)))
}

fn result(r: Result<String, String>) -> Resp {
    match r {
        Ok(m) => ok(&m),
        Err(e) => err(500, &e),
    }
}

fn route(srv: &Server, req: &Req, body: &mut Body, len: u64) -> Resp {
    let ctx = &srv.ctx;
    let m = req.method.as_str();
    let mut form = || -> Vec<(String, String)> {
        let mut b = Vec::new();
        if len <= 8192 {
            let _ = Read::take(&mut *body, 8192).read_to_end(&mut b);
        }
        parse_form(&String::from_utf8_lossy(&b))
    };
    match (m, req.path.as_str()) {
        ("GET" | "HEAD", "/") => Resp {
            status: 200,
            ctype: "text/html; charset=utf-8",
            body: INDEX.as_bytes().to_vec(),
        },
        ("GET", "/api/state") => json(200, state_json(ctx)),
        ("GET", "/api/files") => json(200, files_json()),
        ("GET", "/about") => Resp {
            status: 200,
            ctype: "text/plain; charset=utf-8",
            body: format!(
                "x6100-launcher {}\n\n{LICENSE}\n\n{FONT_NOTICE}\n\nLicences of everything else in this system: /legal/\n",
                env!("CARGO_PKG_VERSION")
            )
            .into_bytes(),
        },
        ("GET", p) if p == "/legal" || p.starts_with("/legal/") => legal(p),
        ("PUT", "/api/upload") => upload(ctx, req, body, len),
        ("POST", "/api/delete") => delete(ctx, req.q("game"), req.q("name")),
        ("POST", "/api/play") => {
            let (tx, rx) = std::sync::mpsc::channel();
            let _ = ctx.tx.send(Msg::Play(req.q("game").to_string(), tx));
            match rx.recv_timeout(Duration::from_secs(10)) {
                Ok(Ok(())) => ok("Starting"),
                Ok(Err(e)) => err(409, &e),
                Err(_) => err(500, "the launcher did not answer"),
            }
        }
        ("POST", "/api/stop") => {
            let _ = ctx.tx.send(Msg::Stop);
            ok("Stopping")
        }
        ("POST", "/api/poweroff") => {
            let _ = ctx.tx.send(Msg::PowerOff);
            ok("Switching off")
        }
        ("POST", "/api/wifi/scan") => match ctx.job("Scanning for Wi-Fi networks", || crate::net::wifi_scan(true)) {
            Ok(nets) => {
                lock(&ctx.shared).networks = nets;
                ctx.wake();
                json(200, state_json(ctx))
            }
            Err(e) => err(500, &e),
        },
        ("POST", "/api/wifi/connect") => {
            let f = form();
            let get = |k: &str| f.iter().find(|(a, _)| a == k).map(|(_, v)| v.clone());
            let ssid = get("ssid").unwrap_or_default();
            let pw = get("password").filter(|p| !p.is_empty());
            if ssid.is_empty() || ssid.len() > 32 {
                return err(400, "no network name");
            }
            if pw.as_ref().is_some_and(|p| p.len() < 8 || p.len() > 63) {
                return err(400, "a WPA password has 8 to 63 characters");
            }
            let saved = lock(&ctx.shared).saved.contains(&ssid);
            let what = format!("Connecting to {ssid}");
            let r = ctx.job(&what, || crate::net::wifi_connect(&ssid, pw.as_deref(), saved));
            if let Ok(m) = &r {
                ctx.note(m.clone(), false);
            }
            result(r)
        }
        ("POST", "/api/wifi/forget") => {
            let f = form();
            let name = f.iter().find(|(a, _)| a == "name").map(|(_, v)| v.clone()).unwrap_or_default();
            if !lock(&ctx.shared).saved.contains(&name) {
                return err(404, "no such saved network");
            }
            result(ctx.job("Forgetting the network", || crate::net::wifi_forget(&name)))
        }
        ("POST", "/api/wifi/radio") => {
            let on = req.q("on") == "1";
            result(ctx.job("Switching Wi-Fi", || crate::net::wifi_enable(on)))
        }
        ("POST", "/api/bt/scan") => match ctx.job("Looking for Bluetooth devices (12 s)", || crate::net::bt_scan(12)) {
            Ok(found) => {
                lock(&ctx.shared).bt_found = found;
                ctx.wake();
                json(200, state_json(ctx))
            }
            Err(e) => err(500, &e),
        },
        ("POST", "/api/bt/do") => {
            let cmd = match req.q("cmd") {
                c @ ("pair" | "connect" | "disconnect" | "forget") => c,
                _ => return err(400, "unknown command"),
            };
            let mac = req.q("mac").to_string();
            if !crate::net::valid_mac(&mac) {
                return err(400, "not a Bluetooth address");
            }
            let r = ctx.job(&format!("Bluetooth: {cmd}"), || crate::net::bt(cmd, Some(&mac)));
            if r.is_ok() && cmd == "pair" {
                lock(&ctx.shared).bt_found.retain(|d| d.mac != mac);
            }
            result(r)
        }
        ("POST", "/api/bt/radio") => {
            let on = req.q("on") == "1";
            result(ctx.job("Switching Bluetooth", || crate::net::bt(if on { "on" } else { "off" }, None)))
        }
        (_, p) if p.starts_with("/api/") => err(404, "no such request"),
        _ => Resp {
            status: 404,
            ctype: "text/plain",
            body: b"Not found\n".to_vec(),
        },
    }
}

fn upload(ctx: &Ctx, req: &Req, body: &mut Body, len: u64) -> Resp {
    let game = req.q("game");
    let name = req.q("name").to_ascii_lowercase();
    if !crate::games::valid_name(game) || !crate::games::valid_name(&name) {
        return err(400, "game and file names: letters, digits, '.', '_', '-'");
    }
    if req.header("Content-Length").is_none() {
        return err(411, "no Content-Length");
    }
    if len > MAX_UPLOAD {
        return err(413, "too large for the FAT partition (4 GiB at most)");
    }
    let home = crate::games::home_dir();
    if let Some(free) = crate::sys::free_bytes(&home) {
        if len + (16 << 20) > free {
            return err(507, &format!("not enough space: {} MB free", free >> 20));
        }
    }
    {
        let mut s = lock(&ctx.shared);
        if s.upload.is_some() {
            return err(409, "another upload is running");
        }
        s.upload = Some(Upload {
            name: format!("{game}/{name}"),
            done: 0,
            total: len,
        });
    }
    ctx.wake();
    let dir = home.join(game);
    let tmp = dir.join(format!(".{name}.part"));
    let r = receive(ctx, body, &dir, &tmp, &dir.join(&name), len);
    lock(&ctx.shared).upload = None;
    if r.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    ctx.refresh();
    match r {
        Ok(()) => {
            eprintln!("launcher: received {game}/{name} ({len} bytes)");
            ctx.note(format!("Received {game}/{name}"), false);
            ok("Received")
        }
        Err(e) => {
            eprintln!("launcher: upload of {game}/{name} failed: {e}");
            err(500, &e.to_string())
        }
    }
}

fn receive(ctx: &Ctx, body: &mut Body, dir: &Path, tmp: &Path, dst: &Path, len: u64) -> io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let mut f = std::fs::File::create(tmp)?;
    let mut buf = vec![0u8; 256 * 1024];
    let mut done = 0u64;
    let mut shown = Instant::now();
    while done < len {
        let n = body.read(&mut buf)?;
        if n == 0 {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "upload cut short"));
        }
        f.write_all(&buf[..n])?;
        done += n as u64;
        if shown.elapsed() > Duration::from_millis(500) {
            shown = Instant::now();
            if let Some(u) = lock(&ctx.shared).upload.as_mut() {
                u.done = done;
            }
            ctx.wake();
        }
    }
    f.sync_all()?;
    drop(f);
    std::fs::rename(tmp, dst)
}

fn delete(ctx: &Ctx, game: &str, name: &str) -> Resp {
    if !crate::games::valid_name(game) || !crate::games::valid_name(name) {
        return err(400, "bad name");
    }
    let p = crate::games::home_dir().join(game).join(name);
    let r = std::fs::remove_file(&p);
    // An empty game directory goes too.
    let _ = std::fs::remove_dir(crate::games::home_dir().join(game));
    ctx.refresh();
    match r {
        Ok(()) => ok("Deleted"),
        Err(e) => err(404, &e.to_string()),
    }
}

/// Files under /usr/share/legal-info (made by Buildroot's legal-info).
fn legal(p: &str) -> Resp {
    let root = path("/usr/share/legal-info");
    let rel = p.trim_start_matches("/legal").trim_matches('/');
    let mut full = root.clone();
    for part in rel.split('/').filter(|s| !s.is_empty()) {
        if part.starts_with('.')
            || !part
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._+-@,".contains(&b))
        {
            return err(404, "no such file");
        }
        full.push(part);
    }
    if full.is_dir() {
        let mut names: Vec<(String, bool)> = std::fs::read_dir(&full)
            .map(|d| {
                d.filter_map(|e| e.ok())
                    .map(|e| (e.file_name().to_string_lossy().to_string(), e.path().is_dir()))
                    .collect()
            })
            .unwrap_or_default();
        names.sort();
        let mut html = String::from(
            "<!doctype html><meta charset=utf-8><meta name=viewport content='width=device-width'>\
             <title>Licences</title><body style='font-family:sans-serif'>",
        );
        if rel.is_empty() {
            html.push_str(
                "<h1>Licences</h1><p>Everything in this system, its version and licence: \
                 <a href='/legal/manifest.csv'>manifest.csv</a>. The texts are below, one \
                 directory per package. The source code is in the legal-info archive that \
                 came with the image (Buildroot's <code>make legal-info</code>).</p>",
            );
            if names.is_empty() {
                html.push_str("<p>This image was built without legal-info.</p>");
            }
        } else {
            html.push_str(&format!("<h1>{}</h1><p><a href='../'>..</a></p>", esc_html(rel)));
        }
        html.push_str("<ul>");
        for (n, dir) in names {
            let slash = if dir { "/" } else { "" };
            html.push_str(&format!(
                "<li><a href='{0}{1}'>{2}{1}</a></li>",
                url_encode(&n),
                slash,
                esc_html(&n)
            ));
        }
        html.push_str("</ul>");
        return Resp {
            status: 200,
            ctype: "text/html; charset=utf-8",
            body: html.into_bytes(),
        };
    }
    match std::fs::read(&full) {
        Ok(b) => Resp {
            status: 200,
            ctype: "text/plain; charset=utf-8",
            body: b,
        },
        Err(_) => err(404, "no such file"),
    }
}

fn state_json(ctx: &Ctx) -> String {
    let s = lock(&ctx.shared).clone();
    let strs = |v: &[String]| format!("[{}]", v.iter().map(|x| js(x)).collect::<Vec<_>>().join(","));
    let opt = |v: &Option<String>| v.as_deref().map_or("null".to_string(), js);
    let dev = |d: &crate::net::BtDevice| {
        format!(
            "{{\"mac\":{},\"name\":{},\"kind\":{},\"connected\":{},\"rssi\":{}}}",
            js(&d.mac),
            js(&d.name),
            js(crate::net::kind_label(&d.kind)),
            d.connected,
            d.rssi.map_or("null".to_string(), |r| r.to_string())
        )
    };
    let list = |v: Vec<String>| format!("[{}]", v.join(","));
    let release = std::fs::read_to_string(path("/etc/x6100-release")).unwrap_or_default();
    format!(
        concat!(
            "{{\"host\":{},\"ips\":{},",
            "\"wifi\":{{\"enabled\":{},\"device\":{},\"state\":{},\"connection\":{}}},",
            "\"saved\":{},\"networks\":{},",
            "\"bt\":{{\"present\":{},\"powered\":{},\"name\":{},\"address\":{}}},",
            "\"bt_paired\":{},\"bt_found\":{},\"games\":{},\"playing\":{},",
            "\"busy\":{},\"note\":{},\"upload\":{},\"free\":{},\"release\":{}}}"
        ),
        js(&s.hostname),
        list(s.ips.iter().map(|(d, ip)| format!("[{},{}]", js(d), js(ip))).collect()),
        s.wifi.enabled,
        opt(&s.wifi.device),
        js(&s.wifi.state),
        opt(&s.wifi.connection),
        strs(&s.saved),
        list(
            s.networks
                .iter()
                .map(|n| format!(
                    "{{\"ssid\":{},\"signal\":{},\"security\":{},\"in_use\":{}}}",
                    js(&n.ssid),
                    n.signal,
                    js(&n.security),
                    n.in_use
                ))
                .collect()
        ),
        s.bt.present,
        s.bt.powered,
        js(&s.bt.name),
        js(&s.bt.address),
        list(s.bt_paired.iter().map(dev).collect()),
        list(s.bt_found.iter().map(dev).collect()),
        list(
            s.games
                .iter()
                .map(|g| format!(
                    "{{\"id\":{},\"title\":{},\"missing\":{}}}",
                    js(&g.id),
                    js(&g.title),
                    strs(&g.missing)
                ))
                .collect()
        ),
        s.playing.as_ref().map_or("null".to_string(), |(g, t)| format!(
            "{{\"game\":{},\"seconds\":{}}}",
            js(g),
            t.elapsed().as_secs()
        )),
        opt(&s.busy),
        s.note.as_ref().map_or("null".to_string(), |(t, e, w)| format!(
            "{{\"text\":{},\"error\":{},\"age\":{}}}",
            js(t),
            e,
            w.elapsed().as_secs()
        )),
        s.upload.as_ref().map_or("null".to_string(), |u| format!(
            "{{\"name\":{},\"done\":{},\"total\":{}}}",
            js(&u.name),
            u.done,
            u.total
        )),
        s.free_home.map_or("null".to_string(), |f| f.to_string()),
        js(release.trim()),
    )
}

fn files_json() -> String {
    let roots: Vec<String> = crate::games::listing()
        .into_iter()
        .map(|(root, writable, files)| {
            let files: Vec<String> = files
                .iter()
                .map(|f| format!("{{\"game\":{},\"name\":{},\"size\":{}}}", js(&f.game), js(&f.name), f.size))
                .collect();
            format!(
                "{{\"path\":{},\"writable\":{},\"files\":[{}]}}",
                js(&root.to_string_lossy()),
                writable,
                files.join(",")
            )
        })
        .collect();
    format!("{{\"roots\":[{}]}}", roots.join(","))
}

fn host_ok(req: &Req, ctx: &Ctx) -> bool {
    let Some(h) = req.header("Host") else {
        return true;
    };
    let h = h.to_ascii_lowercase();
    // Without the port ("[::1]:80", "x6100.local:8080").
    let host = if let Some(rest) = h.strip_prefix('[') {
        return rest.contains(']');
    } else {
        h.rsplit_once(':').map_or(h.as_str(), |(a, _)| a)
    };
    let name = lock(&ctx.shared).hostname.to_ascii_lowercase();
    host.parse::<std::net::Ipv4Addr>().is_ok()
        || host == "localhost"
        || host == name
        || host == format!("{name}.local")
}

fn auth_ok(req: &Req, password: &str) -> bool {
    let Some(v) = req.header("Authorization") else {
        return false;
    };
    let Some(b64) = v.strip_prefix("Basic ") else {
        return false;
    };
    let Some(dec) = base64_decode(b64.trim()) else {
        return false;
    };
    let dec = String::from_utf8_lossy(&dec);
    dec.split_once(':').is_some_and(|(_, p)| p == password)
}

fn respond(s: &mut TcpStream, status: u16, ctype: &str, body: &[u8], extra: &[(&str, &str)]) -> io::Result<()> {
    let reason = match status {
        100 => "Continue",
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        409 => "Conflict",
        411 => "Length Required",
        413 => "Payload Too Large",
        431 => "Request Header Fields Too Large",
        503 => "Service Unavailable",
        507 => "Insufficient Storage",
        _ => "Error",
    };
    let mut h = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\n\
         Connection: close\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\n\
         X-Frame-Options: DENY\r\nReferrer-Policy: no-referrer\r\n",
        body.len()
    );
    for (k, v) in extra {
        h.push_str(&format!("{k}: {v}\r\n"));
    }
    h.push_str("\r\n");
    s.write_all(h.as_bytes())?;
    s.write_all(body)?;
    s.flush()?;
    let _ = s.shutdown(std::net::Shutdown::Write);
    // Let the client read the answer before the socket closes (it may
    // still be sending a body we did not want).
    let mut sink = [0u8; 4096];
    let _ = s.set_read_timeout(Some(Duration::from_secs(2)));
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        match s.read(&mut sink) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
    }
    Ok(())
}

fn find(h: &[u8], needle: &[u8]) -> Option<usize> {
    h.windows(needle.len()).position(|w| w == needle)
}

/// A JSON string.
pub fn js(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 2);
    o.push('"');
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\r' => o.push_str("\\r"),
            '\t' => o.push_str("\\t"),
            // Also '<' etc., so the text can never close a <script>.
            c if (c as u32) < 0x20 || c == '<' || c == '>' || c == '&' => {
                o.push_str(&format!("\\u{:04x}", c as u32))
            }
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

fn esc_html(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('\'', "&#39;")
}

fn url_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < b.len() => {
                match u8::from_str_radix(std::str::from_utf8(&b[i + 1..i + 3]).unwrap_or("zz"), 16) {
                    Ok(v) => {
                        out.push(v);
                        i += 2;
                    }
                    Err(_) => out.push(b'%'),
                }
            }
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

fn url_encode(s: &str) -> String {
    let mut o = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
            o.push(b as char);
        } else {
            o.push_str(&format!("%{b:02X}"));
        }
    }
    o
}

fn parse_form(s: &str) -> Vec<(String, String)> {
    s.split('&')
        .filter(|p| !p.is_empty())
        .map(|p| {
            let (k, v) = p.split_once('=').unwrap_or((p, ""));
            (url_decode(k), url_decode(v))
        })
        .collect()
}

fn base64_decode(s: &str) -> Option<Vec<u8>> {
    let val = |c: u8| -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => (c - b'A') as u32,
            b'a'..=b'z' => (c - b'a' + 26) as u32,
            b'0'..=b'9' => (c - b'0' + 52) as u32,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        })
    };
    let s = s.trim_end_matches('=').as_bytes();
    let mut out = Vec::new();
    let (mut acc, mut bits) = (0u32, 0);
    for &c in s {
        acc = (acc << 6) | val(c)?;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decoding() {
        assert_eq!(url_decode("a%20b+c%2Fd"), "a b c/d");
        assert_eq!(url_decode("100%"), "100%");
        assert_eq!(url_decode("%zz"), "%zz");
        assert_eq!(url_decode("%41"), "A");
        assert_eq!(parse_form("ssid=My+Net&password=p%26w"), vec![
            ("ssid".to_string(), "My Net".to_string()),
            ("password".to_string(), "p&w".to_string())
        ]);
        assert_eq!(base64_decode("dXNlcjpwYXNz").unwrap(), b"user:pass");
        assert_eq!(base64_decode("YQ==").unwrap(), b"a");
    }

    #[test]
    fn json_strings() {
        assert_eq!(js("a\"b\\c\n</script>"), "\"a\\\"b\\\\c\\n\\u003c/script\\u003e\"");
    }
}
