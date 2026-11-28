//! Settings and servers, encrypted (ChaCha20-Poly1305, key readable only by the current user) and written atomically.

use crate::json::Get;
use crate::model::{AppState, Profile, Subscription};
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard, OnceLock};

const MAGIC: &[u8] = b"WRX1";
static STATE: OnceLock<Mutex<AppState>> = OnceLock::new();
static REVISION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// ~/.local/share/wraith (WRAITH_HOME overrides it, for tests and portable setups).
pub fn folder() -> PathBuf {
    if let Some(dir) = std::env::var_os("WRAITH_HOME").map(PathBuf::from).filter(|p| p.is_absolute()) { return dir; }
    crate::util::data_home().join("wraith")
}
fn file() -> PathBuf { folder().join("wraith-state.dat") }

pub fn init() {
    let _ = std::fs::create_dir_all(folder());
    { use std::os::unix::fs::PermissionsExt; let _ = std::fs::set_permissions(folder(), std::fs::Permissions::from_mode(0o700)); }
    let mut state = load();
    if !state.settings.contains_key("ipMode") && state.settings.get("ipv6").and_then(|v| v.as_bool()) == Some(false) { state.set("ipMode", "v4".into()); }
    // 0.11: system proxy and TUN together became the default mode.
    if state.schema < 4 { if state.settings.s("tunnelMode") == "proxy" || state.settings.s("tunnelMode").is_empty() { state.set("tunnelMode", "both".into()); } state.schema = 4; }
    if state.settings.s("hwid").is_empty() { state.set("hwid", crate::util::uuid4().into()); }
    for (k, v) in crate::model::defaults() { state.settings.entry(k).or_insert(v); }
    // 0.12: sing-box is gone. Servers it ran are converted for Xray where possible, otherwise removed.
    if state.profiles.iter().any(|p| p.engine == "singbox") {
        let mut kept = vec![];
        for p in std::mem::take(&mut state.profiles) {
            if p.engine != "singbox" { kept.push(p); continue; }
            match crate::importer::parse(&p.source).ok().and_then(|r| r.profiles.into_iter().next()) {
                Some(n) => kept.push(crate::model::Profile { id: p.id, group: p.group, favorite: p.favorite, ..n }),
                None => crate::log::add(crate::trf!("«{}» удалён: Xray его не поддерживает.", "«{}» removed: Xray does not support it.", p.name)),
            }
        }
        state.profiles = kept;
        if state.selected().is_none() { state.selected = state.profiles.first().map(|p| p.id.clone()).unwrap_or_default(); }
    }
    state.settings.remove("core");
    // 0.13.1: routing is always by rules (the «everything through VPN / direct» modes are gone).
    state.set("mode", "rules".into());
    let _ = STATE.set(Mutex::new(state));
    save();
}

/// Locked state. Keep the guard short: never hold it across network or process waits.
pub fn get() -> MutexGuard<'static, AppState> { STATE.get().expect("store").lock().unwrap_or_else(|e| e.into_inner()) }
/// A setting when the store is loaded (dialogs may run before it), else "".
pub fn try_setting(key: &str) -> String { STATE.get().map(|m| m.lock().unwrap_or_else(|e| e.into_inner()).s(key)).unwrap_or_default() }
/// Changes on every save, so views can refresh cached data.
pub fn revision() -> u64 { REVISION.load(std::sync::atomic::Ordering::Relaxed) }

pub fn update<R>(f: impl FnOnce(&mut AppState) -> R) -> R {
    let r = { let mut s = get(); f(&mut s) };
    save(); r
}

/// The change is visible at once (the revision moves); encrypting and writing happen on a background
/// thread, and changes in quick succession are written once. [flush] writes what is pending (on exit).
pub fn save() {
    REVISION.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    CHANGES.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    if cfg!(test) { write_now(); return; }
    let tx = WRITER.get_or_init(|| {
        let (tx, rx) = std::sync::mpsc::channel::<()>();
        std::thread::spawn(move || while rx.recv().is_ok() {
            std::thread::sleep(std::time::Duration::from_millis(120));
            while rx.try_recv().is_ok() {}
            write_now();
        });
        std::sync::Mutex::new(tx)
    });
    let _ = tx.lock().unwrap_or_else(|e| e.into_inner()).send(());
}

static CHANGES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static WRITTEN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static WRITER: std::sync::OnceLock<std::sync::Mutex<std::sync::mpsc::Sender<()>>> = std::sync::OnceLock::new();
static WRITE_LOCK: Mutex<()> = Mutex::new(());

/// Writes pending changes now; called before the process exits.
pub fn flush() { if STATE.get().is_some() && WRITTEN.load(std::sync::atomic::Ordering::SeqCst) < CHANGES.load(std::sync::atomic::Ordering::SeqCst) { write_now(); } }

fn write_now() {
    let _g = WRITE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let target = CHANGES.load(std::sync::atomic::Ordering::SeqCst);
    if target <= WRITTEN.load(std::sync::atomic::Ordering::SeqCst) && target != 0 { return; }
    let json = { serde_json::to_vec(&*get()).unwrap_or_default() };
    match protect(&json, true) {
        Ok(data) => {
            let tmp = file().with_extension("dat.new");
            if write_private(&tmp, &data).is_ok() { let _ = std::fs::rename(&tmp, file()); }
            WRITTEN.store(target, std::sync::atomic::Ordering::SeqCst);
        }
        Err(e) => crate::log::add(crate::trf!("Не удалось сохранить настройки: {e}", "Could not save the settings: {e}")),
    }
}

pub(crate) fn load() -> AppState {
    let path = file();
    if path.exists() {
        match std::fs::read(&path).map_err(|e| e.to_string()).and_then(|d| protect(&d, false)) {
            Ok(json) => match serde_json::from_slice::<AppState>(&json) {
                Ok(s) => return s,
                Err(e) => backup_broken(&path, &e.to_string()),
            },
            Err(e) => backup_broken(&path, &e),
        }
        return AppState::default();
    }
    AppState::default()
}

/// An unreadable file must never be replaced by an empty state on the next save. Keep a copy.
fn backup_broken(path: &std::path::Path, why: &str) {
    crate::log::add(crate::trf!("Не удалось прочитать настройки: {}", "Could not read the settings: {}", crate::log::safe(why)));
    // Moved aside (not copied): the next save then starts a fresh file, and a fresh key if the old one is gone.
    let _ = std::fs::rename(path, path.with_extension(format!("broken-{}", crate::util::unix_now())));
}

/// Replace one subscription's servers in the provider's order, keeping favorites and the selection by name/host.
pub fn replace_subscription(sub: Subscription, mut fresh: Vec<Profile>) {
    update(|s| {
        let old: Vec<Profile> = s.profiles.iter().filter(|p| p.group == sub.id).cloned().collect();
        let selected_key = s.selected().filter(|p| p.group == sub.id).map(|p| p.key());
        for p in fresh.iter_mut() {
            p.group = sub.id.clone();
            // Old latencies are dropped: a refreshed subscription shows the provider's order until it is pinged again.
            if let Some(prev) = old.iter().find(|o| o.key() == p.key()) { p.id = prev.id.clone(); p.favorite = prev.favorite; }
        }
        let at = s.profiles.iter().position(|p| p.group == sub.id).unwrap_or(s.profiles.len());
        s.profiles.retain(|p| p.group != sub.id);
        let at = at.min(s.profiles.len());
        if let Some(key) = selected_key {
            s.selected = fresh.iter().find(|p| p.key() == key).or(fresh.first()).map(|p| p.id.clone()).unwrap_or_default();
        }
        s.profiles.splice(at..at, fresh);
        match s.subscriptions.iter().position(|x| x.id == sub.id) { Some(i) => s.subscriptions[i] = sub, None => s.subscriptions.push(sub) }
        if s.selected().is_none() { s.selected = s.profiles.first().map(|p| p.id.clone()).unwrap_or_default(); }
    });
}

fn write_private(path: &std::path::Path, data: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(path)?;
    f.write_all(data)?; f.sync_all()
}

/// 32 random bytes in `key` (0600) next to the state; created on first use.
/// Read once per run: every save used to read the key file again.
static KEY: std::sync::OnceLock<[u8; 32]> = std::sync::OnceLock::new();
fn key() -> Result<[u8; 32], String> {
    if let Some(k) = KEY.get() { return Ok(*k); }
    let k = load_key()?; let _ = KEY.set(k); Ok(k)
}
fn load_key() -> Result<[u8; 32], String> {
    let path = folder().join("key");
    if let Ok(k) = std::fs::read(&path) { if k.len() == 32 { let mut out = [0u8; 32]; out.copy_from_slice(&k); return Ok(out); } }
    if file().exists() { return Err(crate::tr!("Ключ настроек потерян (файл key).", "The settings key (file key) is missing.").into()); }
    let mut k = [0u8; 32]; getrandom::getrandom(&mut k).map_err(|e| e.to_string())?;
    write_private(&path, &k).map_err(|e| e.to_string())?;
    Ok(k)
}

/// Replaces DPAPI: authenticated encryption with the per-user key; a tampered or foreign file is refused.
fn protect(data: &[u8], encrypt: bool) -> Result<Vec<u8>, String> {
    use chacha20poly1305::aead::{Aead, KeyInit};
    use chacha20poly1305::{ChaCha20Poly1305, Nonce};
    let cipher = ChaCha20Poly1305::new((&key()?).into());
    if encrypt {
        let mut nonce = [0u8; 12]; getrandom::getrandom(&mut nonce).map_err(|e| e.to_string())?;
        let sealed = cipher.encrypt(Nonce::from_slice(&nonce), data).map_err(|_| "encrypt".to_string())?;
        Ok([MAGIC, &nonce, &sealed].concat())
    } else {
        if data.len() < MAGIC.len() + 12 + 16 || &data[..MAGIC.len()] != MAGIC { return Err(crate::tr!("Неизвестный формат файла настроек.", "Unknown settings file format.").into()); }
        let (nonce, sealed) = data[MAGIC.len()..].split_at(12);
        cipher.decrypt(Nonce::from_slice(nonce), sealed).map_err(|_| crate::tr!("Файл настроек не расшифрован (другой ключ или повреждён).", "The settings file could not be decrypted (another key or damaged).").into())
    }
}

/// Servers, subscriptions, rules and settings as plain JSON (contains connection keys).
pub fn backup() -> String {
    let mut v = serde_json::to_value(&*get()).unwrap_or_default();
    if let Some(s) = v.get_mut("settings").and_then(|s| s.as_object_mut()) { s.remove("hwid"); }
    if let Some(o) = v.as_object_mut() { o.insert("wraithBackup".into(), 1.into()); }
    serde_json::to_string_pretty(&v).unwrap_or_default()
}

/// Replaces everything with a backup; the installation id stays this computer's.
pub fn restore(text: &str) -> Result<(), String> {
    let v: serde_json::Value = serde_json::from_str(text.trim()).map_err(|_| crate::tr!("Это не резервная копия Wraith (JSON не читается).", "This is not a Wraith backup (unreadable JSON).").to_string())?;
    if v.get("profiles").and_then(|p| p.as_array()).is_none() || v.get("settings").and_then(|s| s.as_object()).is_none() {
        return Err(crate::tr!("Это не резервная копия Wraith.", "This is not a Wraith backup.").into());
    }
    let mut state: AppState = serde_json::from_value(v).map_err(|e| e.to_string())?;
    let hwid = get().s("hwid");
    state.set("hwid", hwid.into());
    for (k, v) in crate::model::defaults() { state.settings.entry(k).or_insert(v); }
    state.set("mode", "rules".into());
    if state.selected().is_none() { state.selected = state.profiles.first().map(|p| p.id.clone()).unwrap_or_default(); }
    *get() = state;
    save();
    Ok(())
}
