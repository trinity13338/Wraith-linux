#![recursion_limit = "512"]
//! Wraith for Linux: Xray VPN client.

mod core;
#[macro_use]
mod i18n;
mod importer;
mod install;
mod json;
mod log;
mod model;
mod pinger;
mod routing;
mod setup;
mod store;
mod subs;
mod ui;
mod util;
mod xray;
#[cfg(test)]
mod tests;

fn main() -> eframe::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let has = |a: &str| args.iter().any(|x| x == a);
    if cfg!(feature = "setup") { return setup::run(); }
    if has("--uninstall") { install::uninstall(); return Ok(()); }
    // Before any thread exists: SIGTERM/SIGINT/SIGHUP (logout, kill, Ctrl+C) are taken by one thread
    // that restores the system proxy and stops the core, like a normal exit.
    let signals = block_signals();
    let pending = store::folder().join("restart.pending");
    let wait = has("--elevated") || pending.exists(); let _ = std::fs::remove_file(&pending);
    // One instance: a second launch brings the running window to the front.
    let Some(listener) = ui::instance::acquire(if wait { 50 } else { 1 }) else { ui::instance::signal_running(); return Ok(()); };

    store::init();
    #[cfg(debug_assertions)]
    if has("--demo") { demo(); }
    #[cfg(debug_assertions)]
    if let Some(t) = args.iter().find_map(|a| a.strip_prefix("--theme=")) { let t = t.to_string(); store::update(|s| s.set("theme", t.into())); }
    core::recover();
    install::clean_old_files();
    std::panic::set_hook(Box::new(|info| { log::add(crate::trf!("Сбой: {info}", "Crash: {info}")); core::shutdown(); }));
    std::thread::spawn(move || wait_signals(signals));
    log::add(crate::trf!("Wraith {} запущен", "Wraith {} started", env!("CARGO_PKG_VERSION")));

    ui::instance::listen(listener);
    ui::tray::create();
    ui::app::startup(has("--connect"));
    // The window may close into the tray and open again; the core and the tray outlive it.
    let mut hidden = has("--tray") && ui::tray_available();
    loop {
        if hidden { ui::wait_for_show(); }
        run_window()?;
        if !ui::closing_to_tray() { break; }
        hidden = true;
    }
    core::shutdown();
    Ok(())
}

fn run_window() -> eframe::Result<()> {
    let icon = egui::IconData { rgba: ui::icon::rgba(64), width: 64, height: 64 };
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_title(ui::TITLE).with_app_id(ui::APP_ID).with_inner_size([1140.0, 760.0]).with_min_inner_size([760.0, 560.0]).with_icon(std::sync::Arc::new(icon)),
        centered: true,
        ..Default::default()
    };
    eframe::run_native(ui::TITLE, options, Box::new(move |cc| Ok(Box::new(ui::app::App::new(cc, false)))))
}

fn block_signals() -> libc::sigset_t {
    unsafe {
        let mut set: libc::sigset_t = std::mem::zeroed();
        libc::sigemptyset(&mut set);
        for s in [libc::SIGTERM, libc::SIGINT, libc::SIGHUP] { libc::sigaddset(&mut set, s); }
        libc::pthread_sigmask(libc::SIG_BLOCK, &set, std::ptr::null_mut());
        set
    }
}
fn wait_signals(set: libc::sigset_t) {
    let mut sig = 0;
    if unsafe { libc::sigwait(&set, &mut sig) } != 0 { return; }
    log::add(crate::trf!("Получен сигнал {sig}, выходим.", "Signal {sig} received, exiting."));
    core::shutdown();
    std::process::exit(0);
}

/// Debug builds only: sample servers for UI review.
#[cfg(debug_assertions)]
fn demo() {
    if !store::get().profiles.is_empty() { return; }
    let sub = model::Subscription { id: "demo".into(), name: "My VPN Provider".into(), url: "https://example.com/sub".into(), updated: util::unix_now() - 3600,
        user_info: format!("upload=1073741824; download=16106127360; total=107374182400; expire={}", util::unix_now() + 30 * 86400), announce: crate::tr!("Новые серверы в Финляндии уже доступны", "New servers in Finland are available").into(), format: "links".into(), parsed: subs::PARSE_VERSION, ..Default::default() };
    let links = [
        ("vless://00948d0e-1437-0005-abfe-bb90bf6bc89a@de.example.com:443?type=tcp&security=reality&pbk=AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA&sid=01&sni=www.example.org&flow=xtls-rprx-vision#%F0%9F%87%A9%F0%9F%87%AA%20Germany%20%C2%B7%20Frankfurt", 42),
        ("vless://00948d0e-1437-0005-abfe-bb90bf6bc89a@nl.example.com:443?type=xhttp&security=tls&sni=nl.example.com&path=%2Fx#%F0%9F%87%B3%F0%9F%87%B1%20Netherlands", 58),
        ("trojan://secret@fi.example.com:443?sni=fi.example.com#%F0%9F%87%AB%F0%9F%87%AE%20Finland%20%C2%B7%20Helsinki", -1),
        ("hy2://pass@us.example.com:443?sni=us.example.com#%F0%9F%87%BA%F0%9F%87%B8%20USA%20%C2%B7%20New%20York", 131),
    ];
    let mut profiles = vec![];
    for (i, (link, ms)) in links.iter().enumerate() {
        let mut p = importer::from_link(link).unwrap(); p.group = "demo".into(); p.latency = *ms; p.favorite = i == 1;
        if *ms < 0 { p.ping_error = "timeout".into(); }
        profiles.push(p);
    }
    let mut auto = importer::parse(include_str!("../tests/auto.json")).unwrap().profiles.remove(0); auto.latency = 37;
    profiles.push(auto);
    store::update(|s| { s.subscriptions.push(sub); s.selected = profiles[0].id.clone(); s.profiles = profiles; });
}
