//! Linux helpers: time, child processes, TUN rights (file capabilities), ports, formatting.

use std::path::{Path, PathBuf};
use std::process::Command;

pub struct Now { pub date: String, pub time: String }
pub fn local_time() -> Now {
    let t = unsafe { libc::time(std::ptr::null_mut()) };
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe { libc::localtime_r(&t, &mut tm) };
    Now { date: format!("{:04}-{:02}-{:02}", tm.tm_year + 1900, tm.tm_mon + 1, tm.tm_mday), time: format!("{:02}:{:02}:{:02}", tm.tm_hour, tm.tm_min, tm.tm_sec) }
}
pub fn unix_now() -> i64 { std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0) }

/// Calendar date for a unix time (UTC), enough for "до 12.12.2026".
pub fn date_of(unix: i64) -> String {
    let days = unix.div_euclid(86_400);
    let (y, m, d) = civil(days);
    format!("{d:02}.{m:02}.{y}")
}
pub fn civil_of(days: i64) -> (i64, u32, u32) { civil(days) }
fn civil(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468; let era = z.div_euclid(146_097); let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

pub fn plural(n: i64, one: &str, few: &str, many: &str) -> String {
    if crate::i18n::en() { return if n.abs() == 1 { one } else { many }.to_string(); }
    let n = n.abs(); let (d10, d100) = (n % 10, n % 100);
    let w = if d10 == 1 && d100 != 11 { one } else if (2..=4).contains(&d10) && !(12..=14).contains(&d100) { few } else { many };
    w.to_string()
}
pub fn bytes(v: i64) -> String {
    let v = v.max(0) as f64;
    if v < 1024.0 { crate::trf!("{} Б", "{} B", v as i64) }
    else if v < 1024.0 * 1024.0 { crate::trf!("{:.1} КБ", "{:.1} KB", v / 1024.0) }
    else if v < 1024.0 * 1024.0 * 1024.0 { crate::trf!("{:.1} МБ", "{:.1} MB", v / 1048576.0) }
    else { crate::trf!("{:.2} ГБ", "{:.2} GB", v / 1073741824.0) }
}
pub fn duration(secs: i64) -> String { format!("{:02}:{:02}:{:02}", secs / 3600, (secs / 60) % 60, secs % 60) }

pub fn random_hex(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes]; let _ = getrandom::getrandom(&mut buf);
    buf.iter().map(|b| format!("{b:02x}")).collect()
}
pub fn uuid4() -> String {
    let mut b = [0u8; 16]; let _ = getrandom::getrandom(&mut b);
    b[6] = (b[6] & 0x0f) | 0x40; b[8] = (b[8] & 0x3f) | 0x80;
    let h: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!("{}-{}-{}-{}-{}", &h[0..8], &h[8..12], &h[12..16], &h[16..20], &h[20..32])
}

pub fn free_port() -> u16 {
    std::net::TcpListener::bind(("127.0.0.1", 0)).and_then(|l| l.local_addr()).map(|a| a.port()).unwrap_or(0)
}
pub fn port_open(port: u16) -> bool {
    std::net::TcpStream::connect_timeout(&std::net::SocketAddr::from(([127, 0, 0, 1], port)), std::time::Duration::from_millis(150)).is_ok()
}

pub fn app_dir() -> PathBuf {
    std::env::current_exe().ok().and_then(|p| p.parent().map(|p| p.to_path_buf())).unwrap_or_else(|| ".".into())
}
/// Where geoip.dat / geosite.dat are: next to Wraith (release archive), else the distribution's Xray package.
pub fn asset_dir() -> PathBuf {
    let own = app_dir();
    if own.join("geoip.dat").exists() { return own; }
    for d in ["/usr/share/xray", "/usr/local/share/xray", "/usr/share/v2ray"] { if Path::new(d).join("geoip.dat").exists() { return d.into(); } }
    own
}
/// A child process with Xray's geo files at hand. stdin is closed unless the caller pipes it.
pub fn hidden(exe: &Path) -> Command {
    let mut c = Command::new(exe);
    c.current_dir(app_dir()).env("XRAY_LOCATION_ASSET", asset_dir()).env("xray.location.asset", asset_dir()).stdin(std::process::Stdio::null());
    c
}
/// Runs a program from PATH (or an absolute path) without output, returns whether it succeeded.
pub fn run_quiet(program: &str, args: &[&str]) -> bool {
    Command::new(program).args(args).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status().map(|s| s.success()).unwrap_or(false)
}
/// Output of a program (stdout, trimmed) when it succeeded.
pub fn output(program: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(program).args(args).stdin(std::process::Stdio::null()).stderr(std::process::Stdio::null()).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}
pub fn which(program: &str) -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|paths| std::env::split_paths(&paths).map(|d| d.join(program)).find(|p| p.is_file()))
}

// ------------------------------------------------------------------ TUN rights

/// Capabilities the Xray core needs for TUN: create the interface and its routes (NET_ADMIN),
/// bind its own sockets to the physical interface (NET_RAW for SO_BINDTODEVICE).
pub const CAPS: &str = "cap_net_admin,cap_net_raw,cap_net_bind_service+ep";
const CAP_NET_ADMIN: u32 = 12;

/// TUN can be used: Wraith runs as root, or the core binary carries the NET_ADMIN file capability.
/// (On Windows this was "Wraith runs as administrator".)
pub fn is_admin() -> bool {
    if unsafe { libc::geteuid() } == 0 { return true; }
    has_net_admin(&crate::core::xray_exe())
}
fn has_net_admin(path: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let Ok(c) = std::ffi::CString::new(path.as_os_str().as_bytes()) else { return false };
    let mut buf = [0u8; 24];
    let n = unsafe { libc::getxattr(c.as_ptr(), c"security.capability".as_ptr(), buf.as_mut_ptr() as *mut libc::c_void, buf.len()) };
    if n < 12 { return false; }
    // struct vfs_cap_data: magic_etc, then {permitted, inheritable} for the low 32 capabilities.
    let magic = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]);
    let permitted = u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]);
    let effective = magic & 1 != 0;
    effective && permitted & (1 << CAP_NET_ADMIN) != 0
}

/// Gives the core binary the capabilities for TUN, once, through polkit (asks for the password).
/// Returns false if the user declined or the core is missing.
pub fn grant_tun() -> Result<(), String> {
    let exe = crate::core::xray_exe();
    if !exe.exists() { return Err(crate::trf!("Не найден {}.", "{} not found.", exe.display())); }
    let setcap = ["/usr/bin/setcap", "/usr/sbin/setcap", "/sbin/setcap"].into_iter().find(|p| Path::new(p).exists()).ok_or(crate::tr!("Не найден setcap (пакет libcap).", "setcap not found (libcap package)."))?;
    pkexec(setcap, &[CAPS, &exe.to_string_lossy()])?;
    if is_admin() { Ok(()) } else { Err(crate::tr!("Права для TUN не выданы.", "TUN rights were not granted.").into()) }
}

/// Runs one program as root through pkexec (the desktop's polkit agent asks for the password).
pub fn pkexec(program: &str, args: &[&str]) -> Result<(), String> {
    let Some(pk) = which("pkexec") else { return Err(crate::tr!("Не найден pkexec (polkit). Выполните команду вручную через sudo.", "pkexec (polkit) not found. Run the command manually with sudo.").into()) };
    let out = Command::new(pk).arg(program).args(args).stdin(std::process::Stdio::null()).output().map_err(|e| e.to_string())?;
    match out.status.code() {
        Some(0) => Ok(()),
        // 126: the dialog was dismissed or not authorised; 127: authentication failed.
        Some(126) | Some(127) => Err(crate::tr!("Запрос прав администратора отменён.", "The administrator prompt was cancelled.").into()),
        _ => Err(tail(&String::from_utf8_lossy(&out.stderr), 300)),
    }
}

// ------------------------------------------------------------------ desktop

/// xdg-open, detached; the child is reaped in the background.
fn xdg_open(target: &str) {
    match Command::new("xdg-open").arg(target).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn() {
        Ok(mut child) => { std::thread::spawn(move || { let _ = child.wait(); }); }
        Err(e) => crate::log::add(crate::trf!("Не удалось открыть: {e}", "Could not open: {e}")),
    }
}
pub fn open_url(url: &str) {
    if url.starts_with("https://") || url.starts_with("tg://") || url.starts_with("http://t.me/") { xdg_open(url); }
}
pub fn open_path(p: &Path) { xdg_open(&p.to_string_lossy()); }

pub fn run_hidden(exe: &Path, args: &[&str], timeout_secs: u64) -> Result<String, String> {
    let mut child = hidden(exe).args(args).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()).spawn().map_err(|e| e.to_string())?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(timeout_secs);
    loop {
        if let Ok(Some(_)) = child.try_wait() { break; }
        if std::time::Instant::now() > deadline { let _ = child.kill(); return Err(crate::tr!("Проверка конфигурации не завершилась вовремя.", "The configuration check did not finish in time.").into()); }
        std::thread::sleep(std::time::Duration::from_millis(40));
    }
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    let text = format!("{}\n{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    if out.status.success() { Ok(text) } else { Err(text) }
}

/// Last meaningful lines of a core's output (no banner).
pub fn tail(text: &str, max: usize) -> String {
    let joined = text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with("Xray ") && !l.starts_with("A unified platform")).collect::<Vec<_>>().join(" ");
    if joined.chars().count() > max { format!("…{}", joined.chars().rev().take(max).collect::<Vec<_>>().into_iter().rev().collect::<String>()) } else { joined }
}

/// Clipboard text (Wayland data-control or X11), with wl-paste / xclip as a fallback.
pub fn clipboard_text() -> Option<String> {
    if let Ok(mut c) = arboard::Clipboard::new() { if let Ok(t) = c.get_text() { if !t.is_empty() { return Some(t); } } }
    output("wl-paste", &["--no-newline"]).or_else(|| output("xclip", &["-selection", "clipboard", "-o"])).filter(|t| !t.is_empty())
}

/// Installed by the setup into /opt/wraith (marker written by the installer).
pub fn installed() -> bool { app_dir().join("wraith.installed").exists() }
/// Installed from a distribution package (AUR): pacman owns the files, so updates and removal go through it.
pub fn packaged() -> bool { app_dir().join("wraith.packaged").exists() }
pub const PACKAGE_HINT_RU: &str = "Wraith установлен из пакета. Обновляйте его пакетным менеджером (например, yay -Syu), удаляйте — через pacman -R.";
pub const PACKAGE_HINT_EN: &str = "Wraith was installed from a package. Update it with the package manager (for example yay -Syu), remove it with pacman -R.";

/// Like run_hidden, with the input written to the child's stdin.
pub fn run_hidden_stdin(exe: &Path, args: &[&str], input: &[u8], timeout_secs: u64) -> Result<String, String> {
    use std::io::Write;
    let mut child = hidden(exe).args(args).stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()).spawn().map_err(|e| e.to_string())?;
    if let Some(mut stdin) = child.stdin.take() { stdin.write_all(input).map_err(|e| e.to_string())?; }
    let out = std::sync::Arc::new(std::sync::Mutex::new(None));
    let o2 = out.clone();
    let handle = std::thread::spawn(move || { let r = child.wait_with_output(); *o2.lock().unwrap() = Some(r); });
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(timeout_secs);
    while !handle.is_finished() { if std::time::Instant::now() > deadline { return Err(crate::tr!("Проверка конфигурации не завершилась вовремя.", "The configuration check did not finish in time.").into()); } std::thread::sleep(std::time::Duration::from_millis(30)); }
    let result = out.lock().unwrap().take().ok_or(crate::tr!("Нет ответа ядра", "No answer from the core"))?.map_err(|e| e.to_string())?;
    let text = format!("{}\n{}", String::from_utf8_lossy(&result.stdout), String::from_utf8_lossy(&result.stderr));
    if result.status.success() { Ok(text) } else { Err(text) }
}

// ------------------------------------------------------------------ processes

/// Processes (pid, executable path) whose binary is one of `names` (file names).
pub fn processes(names: &[&str]) -> Vec<(u32, PathBuf)> {
    let mut out = vec![];
    let Ok(dir) = std::fs::read_dir("/proc") else { return out };
    for e in dir.flatten() {
        let Some(pid) = e.file_name().to_str().and_then(|s| s.parse::<u32>().ok()) else { continue };
        // A core started with file capabilities is not dumpable: its exe link is unreadable even for
        // the same user, so the absolute argv[0] Wraith started it with is used instead.
        let exe = match std::fs::read_link(e.path().join("exe")) {
            Ok(p) => PathBuf::from(p.to_string_lossy().trim_end_matches(" (deleted)")), // a replaced binary shows as "… (deleted)"
            Err(_) => match std::fs::read(e.path().join("cmdline")).ok().and_then(|c| c.split(|b| *b == 0).next().map(|a| PathBuf::from(String::from_utf8_lossy(a).into_owned()))) { Some(p) if p.is_absolute() => p, _ => continue },
        };
        if exe.file_name().and_then(|n| n.to_str()).map(|n| names.contains(&n)).unwrap_or(false) { out.push((pid, exe)); }
    }
    out
}
/// SIGTERM, then SIGKILL if the process is still there after `wait_ms`.
pub fn terminate(pid: u32, wait_ms: u64) {
    let pid = pid as libc::pid_t;
    unsafe { libc::kill(pid, libc::SIGTERM) };
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(wait_ms);
    while std::time::Instant::now() < deadline {
        if unsafe { libc::kill(pid, 0) } != 0 { return; }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    unsafe { libc::kill(pid, libc::SIGKILL) };
}

/// Real user id of a process (from /proc/<pid>/status; the /proc entry itself is root-owned for non-dumpable processes).
pub fn process_uid(pid: u32) -> Option<u32> {
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
    status.lines().find_map(|l| l.strip_prefix("Uid:")).and_then(|v| v.split_whitespace().next()).and_then(|v| v.parse().ok())
}
pub fn my_uid() -> u32 { unsafe { libc::getuid() } }

pub fn home() -> PathBuf { std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/tmp")) }
pub fn config_home() -> PathBuf { std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).filter(|p| p.is_absolute()).unwrap_or_else(|| home().join(".config")) }
pub fn data_home() -> PathBuf { std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).filter(|p| p.is_absolute()).unwrap_or_else(|| home().join(".local/share")) }

/// The desktop's file dialog (kdialog on KDE, else zenity). None when cancelled or no dialog tool exists.
/// Blocks: call it from a background thread.
pub fn file_dialog(save: bool, title: &str, name: &str, filter: &[&str]) -> Result<Option<PathBuf>, String> {
    let kde = std::env::var("XDG_CURRENT_DESKTOP").map(|d| d.to_ascii_uppercase().contains("KDE")).unwrap_or(false);
    let start = home().join(name);
    let patterns = filter.join(" ");
    let try_kdialog = || -> Option<Command> {
        which("kdialog")?;
        let mut c = Command::new("kdialog");
        c.arg("--title").arg(title).arg(if save { "--getsavefilename" } else { "--getopenfilename" }).arg(&start);
        if !patterns.is_empty() { c.arg(&patterns); }
        Some(c)
    };
    let try_zenity = || -> Option<Command> {
        which("zenity")?;
        let mut c = Command::new("zenity");
        c.arg("--file-selection").arg(format!("--title={title}"));
        if save { c.arg("--save").arg(format!("--filename={}", start.display())); }
        if !patterns.is_empty() { c.arg(format!("--file-filter={patterns}")); }
        Some(c)
    };
    let cmd = if kde { try_kdialog().or_else(try_zenity) } else { try_zenity().or_else(try_kdialog) };
    let Some(mut cmd) = cmd else { return Err(crate::tr!("Не найден диалог выбора файла: установите zenity или kdialog.", "No file dialog found: install zenity or kdialog.").into()) };
    let out = cmd.stdin(std::process::Stdio::null()).stderr(std::process::Stdio::null()).output().map_err(|e| e.to_string())?;
    let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if !out.status.success() || path.is_empty() { return Ok(None); }
    Ok(Some(PathBuf::from(path)))
}

/// Local "dd.MM, HH:mm" for a unix time.
pub fn local_short(unix: i64) -> String {
    let t = unix as libc::time_t;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe { libc::localtime_r(&t, &mut tm) };
    format!("{:02}.{:02}, {:02}:{:02}", tm.tm_mday, tm.tm_mon + 1, tm.tm_hour, tm.tm_min)
}
/// The local hour (0–23), for the greeting.
pub fn local_hour() -> i32 {
    let t = unsafe { libc::time(std::ptr::null_mut()) };
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe { libc::localtime_r(&t, &mut tm) };
    tm.tm_hour
}
