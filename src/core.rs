//! Runs the Xray core for the selected server; owns the desktop proxy setting and the traffic counters.

use crate::json::{Get, Obj};
use crate::model::Profile;
use crate::util;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum State { Off, Connecting, Connected, Stopping }

#[derive(Clone)]
pub struct Status {
    pub state: State,
    pub error: String,
    pub profile_id: String,
    pub profile_name: String,
    pub engine: &'static str,
    pub tun: bool,
    pub since: Option<Instant>,
    pub rx: i64, pub tx: i64, pub rx_rate: i64, pub tx_rate: i64,
}
static STATUS: Mutex<Status> = Mutex::new(Status { state: State::Off, error: String::new(), profile_id: String::new(), profile_name: String::new(), engine: "Xray", tun: false, since: None, rx: 0, tx: 0, rx_rate: 0, tx_rate: 0 });
pub fn status() -> Status { STATUS.lock().unwrap().clone() }
fn set_state(state: State, error: Option<String>) {
    let mut s = STATUS.lock().unwrap(); s.state = state; if let Some(e) = error { s.error = e; }
}

struct Running { child: Arc<Mutex<Child>>, proxy_mode: bool, stats_port: u16, tags: Vec<String> }
static RUNNING: Mutex<Option<Running>> = Mutex::new(None);
static SESSION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static CONNECT: Mutex<()> = Mutex::new(());
/// Last profile started and how many automatic restarts it already had.
static LAST: Mutex<Option<Profile>> = Mutex::new(None);
static RETRIES: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
/// Last meaningful line the core printed (shown when it dies).
static LAST_LINE: Mutex<String> = Mutex::new(String::new());

/// The core next to Wraith (release archive), else the distribution's package.
pub fn xray_exe() -> PathBuf {
    let own = util::app_dir().join("xray");
    if own.exists() { return own; }
    ["/usr/bin/xray", "/usr/local/bin/xray"].iter().map(PathBuf::from).find(|p| p.exists()).unwrap_or(own)
}


/// At startup: a crashed Wraith may have left the desktop proxy pointing at a dead port, or a core holding the TUN.
pub fn recover() {
    proxy::restore_leftover();
    kill_orphans();
}

pub fn connect(profile: Profile) { RETRIES.store(0, std::sync::atomic::Ordering::SeqCst); start_connect(profile); }
fn start_connect(profile: Profile) {
    *LAST.lock().unwrap() = Some(profile.clone());
    LAST_LINE.lock().unwrap().clear();
    std::thread::spawn(move || {
        let _serial = CONNECT.lock().unwrap();
        stop_running(false);
        let session = SESSION.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
        {
            let mut s = STATUS.lock().unwrap();
            *s = Status { state: State::Connecting, error: String::new(), profile_id: profile.id.clone(), profile_name: profile.name.clone(), engine: "Xray",tun: crate::model::uses_tun(&crate::store::get().settings), since: None, rx: 0, tx: 0, rx_rate: 0, tx_rate: 0 };
        }
        match start(&profile, session) {
            Ok(()) => { let mut s = STATUS.lock().unwrap(); s.state = State::Connected; s.since = Some(Instant::now()); }
            Err(e) => { let e = crate::log::safe(e); crate::log::add(crate::trf!("Подключение не удалось: {e}", "Connection failed: {e}")); stop_running(true); set_state(State::Off, Some(e)); }
        }
    });
}
pub fn disconnect() {
    std::thread::spawn(|| {
        let _serial = CONNECT.lock().unwrap();
        if RUNNING.lock().unwrap().is_none() { set_state(State::Off, Some(String::new())); return; }
        set_state(State::Stopping, Some(String::new()));
        stop_running(false);
        set_state(State::Off, None);
    });
}
/// Synchronous stop for application exit: the desktop proxy must never stay pointed at a dead port.
pub fn shutdown() { stop_running(true); }

fn start(profile: &Profile, session: u64) -> Result<(), String> {
    let state = crate::store::get().clone();
    let settings = &state.settings;
    let tun = crate::model::uses_tun(settings); let system_proxy = crate::model::uses_proxy(settings);
    if tun && !util::is_admin() { return Err(crate::tr!("Для режима TUN ядру нужны права на управление сетью.", "TUN mode needs network rights for the core.").into()); }
    // The system's resolvers are read before the tunnel exists.
    let system_dns = crate::routing::system_resolvers();
    let stats_port = util::free_port();
    let built = crate::xray::build(&state, profile, &system_dns, stats_port)?;
    let (config, tags) = (built.config, built.proxy_tags);
    let exe = xray_exe();
    if !exe.exists() { return Err(crate::tr!("Не найдено ядро xray рядом с wraith. Распакуйте архив целиком или установите пакет xray.", "The xray core was not found next to wraith. Unpack the whole archive or install the xray package.").into()); }
    // The config goes through stdin: no file another program could swap while the core starts
    // (the core runs with network capabilities).
    let bytes = serde_json::to_vec(&config).unwrap();
    let check = util::run_hidden_stdin(&exe, &["run", "-test", "-c", "stdin:"], &bytes, 20);
    if let Err(out) = check { return Err(crate::trf!("{} отклонил конфигурацию: {}", "{} rejected the configuration: {}", "Xray", util::tail(&out, 600))); }
    crate::log::add(crate::trf!("Wraith: запуск {} · {} · «{}»", "Wraith: starting {} · {} · «{}»", "Xray", if tun && system_proxy { crate::tr!("TUN + прокси", "TUN + proxy") } else if tun { "TUN" } else { crate::tr!("системный прокси", "system proxy") }, profile.name));
    let mut cmd = util::hidden(&exe);
    cmd.args(["run", "-c", "stdin:"]);
    let mut child = cmd.stdout(Stdio::piped()).stderr(Stdio::piped()).stdin(Stdio::piped()).spawn().map_err(|e| crate::trf!("Не удалось запустить ядро: {e}", "Could not start the core: {e}"))?;
    if let Some(mut stdin) = child.stdin.take() { use std::io::Write; stdin.write_all(&bytes).map_err(|e| e.to_string())?; }
    for stream in [child.stdout.take().map(|s| Box::new(s) as Box<dyn std::io::Read + Send>), child.stderr.take().map(|s| Box::new(s) as Box<dyn std::io::Read + Send>)].into_iter().flatten() {
        std::thread::spawn(move || { for line in BufReader::new(stream).lines().map_while(Result::ok) { if !line.contains(" accepted ") && !line.trim().is_empty() { *LAST_LINE.lock().unwrap() = line.clone(); } crate::log::add(line); } });
    }
    let child = Arc::new(Mutex::new(child));
    *RUNNING.lock().unwrap() = Some(Running { child: child.clone(), proxy_mode: system_proxy, stats_port, tags });
    // Wait for the local inlet: it proves the core finished starting (TUN comes up first).
    let port = settings.i("httpProxyPort", 2082) as u16;
    let deadline = Instant::now() + Duration::from_secs(if tun { 12 } else { 6 });
    loop {
        if let Ok(Some(_)) = child.lock().unwrap().try_wait() { return Err(crate::tr!("Ядро остановилось при запуске. Подробности — в журнале.", "The core stopped while starting. See the log.").into()); }
        if util::port_open(port) { break; }
        if Instant::now() > deadline { return Err(crate::trf!("Ядро не открыло локальный порт {port}. Возможно, он занят другой программой.", "The core did not open local port {port}. Another program may be using it.")); }
        std::thread::sleep(Duration::from_millis(80));
    }
    if system_proxy { proxy::enable(&format!("127.0.0.1:{port}"))?; }
    // Watchdog and traffic counters for this session.
    std::thread::spawn(move || monitor(session));
    crate::log::add(crate::tr!("Wraith: подключено", "Wraith: connected"));
    Ok(())
}

fn monitor(session: u64) {
    loop {
        std::thread::sleep(Duration::from_secs(1));
        if SESSION.load(std::sync::atomic::Ordering::SeqCst) != session { return; }
        let (child, port, tags) = {
            let guard = RUNNING.lock().unwrap();
            let Some(r) = guard.as_ref() else { return };
            (r.child.clone(), r.stats_port, r.tags.clone())
        };
        let exited = child.lock().unwrap().try_wait().ok().flatten();
        if let Some(code) = exited {
            let _serial = CONNECT.lock().unwrap();
            if SESSION.load(std::sync::atomic::Ordering::SeqCst) != session { return; }
            crate::log::add(crate::trf!("Ядро неожиданно остановилось ({code}).", "The core stopped unexpectedly ({code})."));
            stop_running(true);
            // A crash of the core (driver hiccup, network change) gets two automatic restarts.
            let retries = RETRIES.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let last = LAST.lock().unwrap().clone();
            if retries < 2 { if let Some(p) = last { crate::log::add(crate::tr!("Перезапускаем ядро…", "Restarting the core…")); drop(_serial); start_connect(p); return; } }
            let line = crate::util::tail(&LAST_LINE.lock().unwrap(), 200);
            set_state(State::Off, Some(if line.is_empty() { crate::tr!("Ядро остановилось. Подробности — в журнале.", "The core stopped. See the log.").into() } else { crate::trf!("Ядро остановилось: {}", "The core stopped: {}", crate::log::safe(line)) }));
            return;
        }
        if let Some((down, up)) = read_stats(port, &tags) {
            let mut s = STATUS.lock().unwrap();
            s.rx_rate = (down - s.rx).max(0); s.tx_rate = (up - s.tx).max(0); s.rx = down; s.tx = up;
        }
        // Local statistics (speed chart, daily totals) and the exit IP, also while the window is in the tray.
        let (rx_rate, tx_rate) = { let s = STATUS.lock().unwrap(); (s.rx_rate, s.tx_rate) };
        crate::stats::sample(rx_rate, tx_rate);
        let settings = crate::store::get().settings.clone();
        crate::stats::tick_exit(&settings);
    }
}

fn read_stats(port: u16, tags: &[String]) -> Option<(i64, i64)> {
    let agent = ureq::AgentBuilder::new().timeout(Duration::from_millis(800)).build();
    let o: Obj = agent.get(&format!("http://127.0.0.1:{port}/debug/vars")).call().ok()?.into_json().ok()?;
    let outbound = o.o("stats").and_then(|s| s.o("outbound"))?;
    let (mut d, mut u) = (0, 0);
    for t in tags { if let Some(x) = outbound.o(t) { d += x.i("downlink", 0); u += x.i("uplink", 0); } }
    Some((d, u))
}

fn stop_running(quiet: bool) {
    SESSION.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let Some(r) = RUNNING.lock().unwrap().take() else { return };
    if r.proxy_mode { if let Err(e) = proxy::restore() { crate::log::add(crate::trf!("Не удалось вернуть настройки системного прокси: {e}", "Could not restore the system proxy settings: {e}")); } }
    { let mut c = r.child.lock().unwrap(); let _ = c.kill(); let _ = c.wait(); }
    if !quiet { crate::log::add(crate::tr!("Wraith: отключено", "Wraith: disconnected")); }
    { let mut s = STATUS.lock().unwrap(); s.rx_rate = 0; s.tx_rate = 0; }
    crate::stats::session_ended(); crate::stats::reset_exit();
}

/// A core left running by a crashed Wraith keeps the TUN and the ports busy.
fn kill_orphans() {
    let ours = xray_exe();
    for (pid, exe) in util::processes(&["xray", "sing-box"]) {
        let ours_too = exe == ours || exe == util::app_dir().join("sing-box");
        // Only our own user's cores, started from Wraith's folder (a system-wide xray service is left alone).
        if !ours_too || !owned_by_me(pid) || (!exe.starts_with(util::app_dir()) && !started_by_wraith(pid)) { continue; }
        util::terminate(pid, 2000);
        crate::log::add(crate::tr!("Остановлено ядро, оставшееся от прошлого запуска.", "Stopped a core left over from the previous run."));
    }
}
fn owned_by_me(pid: u32) -> bool { util::process_uid(pid) == Some(util::my_uid()) }
/// A distribution-packaged core counts as ours only when it runs Wraith's stdin config.
fn started_by_wraith(pid: u32) -> bool {
    std::fs::read(format!("/proc/{pid}/cmdline")).map(|c| c.split(|b| *b == 0).any(|a| a == b"stdin:")).unwrap_or(false)
}

/// The desktop's proxy setting (KDE kioslaverc and/or GNOME gsettings) with a persisted snapshot,
/// restored even after a crash. Like the Windows edition, one HTTP proxy serves HTTP and HTTPS.
pub mod proxy {
    use crate::json::Get;
    use crate::util;
    use serde_json::{json, Value};
    fn backup() -> std::path::PathBuf { crate::store::folder().join("proxy-backup.json") }
    const BYPASS: &[&str] = &["localhost", "127.0.0.0/8", "::1", "10.0.0.0/8", "172.16.0.0/12", "192.168.0.0/16", "*.local"];

    fn kde_tool(kind: &str) -> Option<&'static str> {
        let list: &[&'static str] = if kind == "write" { &["kwriteconfig6", "kwriteconfig5"] } else { &["kreadconfig6", "kreadconfig5"] };
        list.iter().copied().find(|t| util::which(t).is_some())
    }
    fn kde() -> bool { kde_tool("write").is_some() && kde_tool("read").is_some() }
    fn gnome() -> bool { util::which("gsettings").is_some() && util::output("gsettings", &["get", "org.gnome.system.proxy", "mode"]).is_some() }

    fn kread(key: &str) -> Option<String> {
        let t = kde_tool("read")?;
        util::output(t, &["--file", "kioslaverc", "--group", "Proxy Settings", "--key", key])
    }
    fn kwrite(key: &str, value: &str) -> bool {
        let Some(t) = kde_tool("write") else { return false };
        util::run_quiet(t, &["--file", "kioslaverc", "--group", "Proxy Settings", "--key", key, value])
    }
    fn kdelete(key: &str) { if let Some(t) = kde_tool("write") { util::run_quiet(t, &["--file", "kioslaverc", "--group", "Proxy Settings", "--key", key, "--delete"]); } }
    fn gget(schema: &str, key: &str) -> Option<String> { util::output("gsettings", &["get", schema, key]) }
    fn gset(schema: &str, key: &str, value: &str) -> bool { util::run_quiet("gsettings", &["set", schema, key, value]) }

    /// "127.0.0.1:2082" → KDE's "http://127.0.0.1 2082".
    fn kde_value(server: &str) -> String { let (h, p) = server.rsplit_once(':').unwrap_or((server, "")); format!("http://{h} {p}") }

    pub fn enable(server: &str) -> Result<(), String> {
        let (k, g) = (kde(), gnome());
        if !k && !g { return Err(crate::tr!("Не найдено, где менять системный прокси (нужны KDE или GNOME). Используйте TUN или пропишите прокси вручную.", "No desktop proxy setting was found (KDE or GNOME is needed). Use TUN or set the proxy manually.").into()); }
        if !backup().exists() {
            let mut snapshot = json!({"ours": server});
            if k { snapshot["kde"] = json!({"ProxyType": kread("ProxyType"), "httpProxy": kread("httpProxy"), "httpsProxy": kread("httpsProxy"), "NoProxyFor": kread("NoProxyFor")}); }
            if g {
                let mut o = serde_json::Map::new();
                for (schema, keys) in [("org.gnome.system.proxy", &["mode", "ignore-hosts"][..]), ("org.gnome.system.proxy.http", &["host", "port"][..]), ("org.gnome.system.proxy.https", &["host", "port"][..])] {
                    for key in keys { o.insert(format!("{schema} {key}"), gget(schema, key).map(Value::from).unwrap_or(Value::Null)); }
                }
                snapshot["gnome"] = Value::Object(o);
            }
            std::fs::write(backup(), snapshot.to_string()).map_err(|e| e.to_string())?;
        }
        let (host, port) = server.rsplit_once(':').unwrap_or((server, "0"));
        let mut ok = false;
        if k {
            ok |= kwrite("httpProxy", &kde_value(server)) & kwrite("httpsProxy", &kde_value(server)) & kwrite("NoProxyFor", &BYPASS.join(",")) & kwrite("ProxyType", "1");
            refresh_kde();
        }
        if g {
            let ignore = format!("[{}]", BYPASS.iter().map(|h| format!("'{h}'")).collect::<Vec<_>>().join(", "));
            ok |= gset("org.gnome.system.proxy.http", "host", &format!("'{host}'")) & gset("org.gnome.system.proxy.http", "port", port)
                & gset("org.gnome.system.proxy.https", "host", &format!("'{host}'")) & gset("org.gnome.system.proxy.https", "port", port)
                & gset("org.gnome.system.proxy", "ignore-hosts", &ignore) & gset("org.gnome.system.proxy", "mode", "'manual'");
        }
        if ok { Ok(()) } else { Err(crate::tr!("Не удалось включить системный прокси.", "Could not turn on the system proxy.").into()) }
    }

    pub fn restore() -> Result<(), String> {
        let path = backup(); if !path.exists() { return Ok(()); }
        let snapshot: Value = serde_json::from_str(&std::fs::read_to_string(&path).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
        let ours = snapshot.s("ours");
        // Only undo our own change: if the user switched to another proxy meanwhile, leave it.
        if let Some(old) = snapshot.get("kde").and_then(|v| v.as_object()) {
            let current = kread("httpProxy").unwrap_or_default();
            if current.is_empty() || current == kde_value(&ours) {
                for key in ["httpProxy", "httpsProxy", "NoProxyFor", "ProxyType"] {
                    match old.get(key).and_then(|v| v.as_str()).filter(|v| !v.is_empty()) { Some(v) => { kwrite(key, v); } None => kdelete(key) }
                }
                if old.get("ProxyType").and_then(|v| v.as_str()).map(|v| v.is_empty()).unwrap_or(true) { kwrite("ProxyType", "0"); }
                refresh_kde();
            }
        }
        if let Some(old) = snapshot.get("gnome").and_then(|v| v.as_object()) {
            let (host, port) = ours.rsplit_once(':').unwrap_or((&ours, ""));
            let current = (gget("org.gnome.system.proxy.http", "host").unwrap_or_default(), gget("org.gnome.system.proxy.http", "port").unwrap_or_default());
            let mode = gget("org.gnome.system.proxy", "mode").unwrap_or_default();
            if mode != "'manual'" || (current.0 == format!("'{host}'") && current.1 == port) {
                // mode last, so the old setting never points at our port for a moment.
                for (k, v) in old.iter().filter(|(k, _)| !k.ends_with(" mode")).chain(old.iter().filter(|(k, _)| k.ends_with(" mode"))) {
                    let Some((schema, key)) = k.split_once(' ') else { continue };
                    match v.as_str() { Some(v) => { gset(schema, key, v); } None => { util::run_quiet("gsettings", &["reset", schema, key]); } }
                }
            }
        }
        let _ = std::fs::remove_file(path); Ok(())
    }
    pub fn restore_leftover() {
        if backup().exists() { match restore() { Ok(()) => crate::log::add(crate::tr!("Настройки системного прокси восстановлены после прошлого запуска.", "System proxy settings restored after the previous run.")), Err(e) => crate::log::add(crate::trf!("Не удалось восстановить системный прокси: {e}", "Could not restore the system proxy: {e}")) } }
    }
    /// KIO-based apps re-read the setting on this signal; others read it when they start a request.
    fn refresh_kde() {
        util::run_quiet("dbus-send", &["--type=signal", "/KIO/Scheduler", "org.kde.KIO.Scheduler.reparseSlaveConfiguration", "string:"]);
    }
}

/// Tests: draw the connected screens without a core.
#[cfg(test)]
pub fn set_test_status(state: State, name: &str, rx_rate: i64, tx_rate: i64) {
    let mut s = STATUS.lock().unwrap();
    *s = Status { state, error: String::new(), profile_id: String::new(), profile_name: name.into(), engine: "Xray", tun: true, since: Some(Instant::now() - Duration::from_secs(3725)), rx: 734_003_200, tx: 52_428_800, rx_rate, tx_rate };
}
