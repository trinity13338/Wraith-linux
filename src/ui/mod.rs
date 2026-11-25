//! Window handling (show/hide from the tray), tray icon, single instance and small dialogs.
//!
//! Wayland lets no client hide its own window, so "minimise to the tray" closes the window while the
//! core keeps running; the tray (or a second launch) opens a fresh one. eframe runs again on the same
//! event loop, and all state lives outside the window.

pub mod app;
pub mod backdrop;
pub mod dialog;
pub mod flags;
pub mod icon;
pub mod icons;
pub mod theme;
pub mod widgets;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex};

static CTX: Mutex<Option<egui::Context>> = Mutex::new(None);
static TRAY: AtomicBool = AtomicBool::new(false);
static OPEN: AtomicBool = AtomicBool::new(false);
static TO_TRAY: AtomicBool = AtomicBool::new(false);
static SHOW: (Mutex<bool>, Condvar) = (Mutex::new(false), Condvar::new());
pub const TITLE: &str = "Wraith";
pub const APP_ID: &str = "wraith";

pub fn set_context(ctx: egui::Context) { *CTX.lock().unwrap() = Some(ctx); OPEN.store(true, Ordering::SeqCst); TO_TRAY.store(false, Ordering::SeqCst); }
pub fn context() -> Option<egui::Context> { CTX.lock().unwrap().clone() }
pub fn repaint() { if let Some(c) = context() { c.request_repaint(); } }
pub fn tray_available() -> bool { TRAY.load(Ordering::Relaxed) }

/// Called once the first frames are drawn.
pub fn window_ready(start_hidden: bool) { if start_hidden && tray_available() { hide_window(); } }

/// Closes the window into the tray (the app keeps running).
pub fn hide_window() {
    TO_TRAY.store(true, Ordering::SeqCst);
    if let Some(c) = context() { c.send_viewport_cmd(egui::ViewportCommand::Close); c.request_repaint(); }
}
/// The window is closing into the tray rather than quitting.
pub fn closing_to_tray() -> bool { TO_TRAY.load(Ordering::SeqCst) }
pub fn window_closed() { OPEN.store(false, Ordering::SeqCst); *CTX.lock().unwrap() = None; }

pub fn show_window() {
    if OPEN.load(Ordering::SeqCst) {
        if let Some(c) = context() {
            c.send_viewport_cmd(egui::ViewportCommand::Visible(true));
            c.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
            c.send_viewport_cmd(egui::ViewportCommand::Focus);
            c.request_repaint();
        }
        return;
    }
    let (lock, cv) = &SHOW; *lock.lock().unwrap() = true; cv.notify_all();
}
/// Blocks the main thread while the window is in the tray.
pub fn wait_for_show() {
    let (lock, cv) = &SHOW;
    let mut shown = lock.lock().unwrap();
    while !*shown { shown = cv.wait(shown).unwrap(); }
    *shown = false;
}

/// One Wraith per user: an abstract Unix socket. A second launch asks the running one to show its window.
pub mod instance {
    use std::io::{Read, Write};
    use std::os::linux::net::SocketAddrExt;
    use std::os::unix::net::{SocketAddr, UnixListener, UnixStream};

    fn name() -> String { format!("wraith-{}{}", crate::util::my_uid(), if cfg!(debug_assertions) { "-debug" } else { "" }) }

    /// Some(listener) when this process is the only instance; tries `attempts` times 200 ms apart.
    pub fn acquire(attempts: u32) -> Option<UnixListener> {
        let addr = SocketAddr::from_abstract_name(name().as_bytes()).ok()?;
        for _ in 0..attempts.max(1) {
            if let Ok(l) = UnixListener::bind_addr(&addr) { return Some(l); }
            std::thread::sleep(std::time::Duration::from_millis(200));
        }
        None
    }
    /// Tells the running instance to show its window.
    pub fn signal_running() -> bool {
        let Ok(addr) = SocketAddr::from_abstract_name(name().as_bytes()) else { return false };
        UnixStream::connect_addr(&addr).and_then(|mut s| s.write_all(b"show")).is_ok()
    }
    pub fn listen(listener: UnixListener) {
        std::thread::spawn(move || for stream in listener.incoming().flatten() {
            let mut s = stream; let mut buf = [0u8; 16];
            let _ = s.set_read_timeout(Some(std::time::Duration::from_secs(1)));
            if let Ok(n) = s.read(&mut buf) { if &buf[..n] == b"show" { super::show_window(); } }
        });
    }
}

pub mod tray {
    use ksni::menu::{MenuItem, StandardItem};
    use std::sync::atomic::{AtomicBool, Ordering};

    /// The tray host answered the first registration (online or not).
    static ANSWERED: AtomicBool = AtomicBool::new(false);
    struct Tray;

    fn toggle() {
        let st = crate::core::status();
        if st.state == crate::core::State::Off {
            let selected = crate::store::get().selected().cloned();
            let tun = crate::model::uses_tun(&crate::store::get().settings);
            match selected { Some(p) if !tun || crate::util::is_admin() => crate::core::connect(p), _ => super::show_window() }
        } else { crate::core::disconnect(); }
        super::repaint();
    }

    impl ksni::Tray for Tray {
        fn id(&self) -> String { super::APP_ID.into() }
        fn title(&self) -> String { "Wraith".into() }
        fn icon_name(&self) -> String { String::new() }
        /// The mark alone, light on a dark panel and dark on a light one, in every size panels ask for.
        fn icon_pixmap(&self) -> Vec<ksni::Icon> {
            let rgb = if super::dark_panel() { [0xF5, 0xF2, 0xFA] } else { [0x1C, 0x1B, 0x22] };
            [16u32, 22, 24, 32, 48, 64].iter().map(|&s| {
                // RGBA → ARGB32 in network byte order.
                let data = super::icon::mark(s, rgb).chunks(4).flat_map(|p| [p[3], p[0], p[1], p[2]]).collect();
                ksni::Icon { width: s as i32, height: s as i32, data }
            }).collect()
        }
        fn tool_tip(&self) -> ksni::ToolTip {
            let st = crate::core::status();
            let text = match st.state {
                crate::core::State::Connected => crate::trf!("Подключено · {}\n↓ {}/с  ↑ {}/с", "Connected · {}\n↓ {}/s  ↑ {}/s", crate::ui::widgets::display_name(&st.profile_name), crate::util::bytes(st.rx_rate), crate::util::bytes(st.tx_rate)),
                _ => crate::tr!("Не подключено", "Not connected").into(),
            };
            ksni::ToolTip { title: "Wraith".into(), description: text, ..Default::default() }
        }
        // Left click: the window, like the Windows tray icon.
        fn activate(&mut self, _x: i32, _y: i32) { super::show_window(); }
        fn menu(&self) -> Vec<MenuItem<Self>> {
            vec![
                StandardItem { label: crate::tr!("Открыть Wraith", "Open Wraith").into(), activate: Box::new(|_: &mut Self| super::show_window()), ..Default::default() }.into(),
                StandardItem { label: crate::tr!("Подключить / отключить", "Connect / disconnect").into(), activate: Box::new(|_: &mut Self| toggle()), ..Default::default() }.into(),
                MenuItem::Separator,
                StandardItem { label: crate::tr!("Выход", "Quit").into(), activate: Box::new(|_: &mut Self| { crate::core::shutdown(); std::process::exit(0); }), ..Default::default() }.into(),
            ]
        }
        fn watcher_online(&self) { super::TRAY.store(true, Ordering::Relaxed); ANSWERED.store(true, Ordering::Relaxed); }
        // No tray host (e.g. GNOME without the AppIndicator extension): keep waiting for one.
        fn watcher_offine(&self) -> bool { super::TRAY.store(false, Ordering::Relaxed); ANSWERED.store(true, Ordering::Relaxed); true }
    }

    /// Starts the tray on its own D-Bus thread; waits briefly to learn whether a tray host exists.
    pub fn create() {
        let service = ksni::TrayService::new(Tray);
        let handle = service.handle();
        std::thread::spawn(move || {
            if let Err(e) = service.run() { super::TRAY.store(false, Ordering::Relaxed); ANSWERED.store(true, Ordering::Relaxed); crate::log::add(crate::trf!("Значок в трее недоступен: {e}", "Tray icon unavailable: {e}")); }
        });
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(1500);
        while !ANSWERED.load(Ordering::Relaxed) && std::time::Instant::now() < deadline { std::thread::sleep(std::time::Duration::from_millis(20)); }
        // The tooltip follows the connection state.
        std::thread::spawn(move || loop { std::thread::sleep(std::time::Duration::from_secs(2)); handle.update(|_| {}); });
    }
}

/// The panel (tray) is dark: GNOME's top bar always is; KDE and others follow the colour scheme.
pub fn dark_panel() -> bool {
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default().to_ascii_uppercase();
    if ["GNOME", "UNITY", "PANTHEON", "BUDGIE"].iter().any(|d| desktop.contains(d)) { return true; }
    if let Ok(text) = std::fs::read_to_string(crate::util::config_home().join("kdeglobals")) {
        // Plasma panels use the Window colours of the scheme.
        let mut in_window = false;
        for line in text.lines() {
            let l = line.trim();
            if l.starts_with('[') { in_window = l == "[Colors:Window]"; continue; }
            if in_window { if let Some(rgb) = l.strip_prefix("BackgroundNormal=") {
                let c: Vec<f32> = rgb.split(',').filter_map(|x| x.trim().parse().ok()).collect();
                if c.len() >= 3 { return 0.299 * c[0] + 0.587 * c[1] + 0.114 * c[2] < 128.0; }
            } }
        }
    }
    match crate::util::output("gsettings", &["get", "org.gnome.desktop.interface", "color-scheme"]) {
        Some(v) if v.contains("prefer-light") => false,
        Some(v) if v.contains("default") => !crate::util::output("gsettings", &["get", "org.gnome.desktop.interface", "gtk-theme"]).map(|t| !t.to_ascii_lowercase().contains("dark")).unwrap_or(false),
        _ => true,
    }
}

/// The app icon in the interface's accent (the system accent by default), for windows and menus.
pub fn app_icon(size: u32) -> egui::IconData {
    let [r, g, b, _] = theme::accent_color().to_srgba_unmultiplied();
    egui::IconData { rgba: icon::tile(size, [0x0B, 0x0A, 0x10], [r, g, b]), width: size, height: size }
}
