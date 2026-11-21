//! Installation into /opt/wraith, desktop entries, autostart, uninstall, and unpacking release
//! archives (used by the installer and by the updater).
//!
//! The installed copy is root-owned, so nothing the user runs can swap the core that carries the
//! network capabilities. Everything that needs root happens in one pkexec step per install/update.

use crate::util;
use std::io::Read;
use std::path::{Path, PathBuf};

pub fn install_dir() -> PathBuf { PathBuf::from("/opt/wraith") }
const SYSTEM_DESKTOP: &str = "/usr/share/applications/wraith.desktop";
const SYSTEM_ICON: &str = "/usr/share/icons/hicolor/256x256/apps/wraith.png";
const SYSTEM_LINK: &str = "/usr/local/bin/wraith";

/// Unpacks a release zip into `dir`. A single top folder in the archive is stripped. A file that is
/// in use (the running wraith) is renamed to *.old first; *.old files are removed on next start.
pub fn extract(zip_bytes: &[u8], dir: &Path, progress: &dyn Fn(usize, usize)) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(zip_bytes)).map_err(|e| crate::trf!("Архив повреждён: {e}", "The archive is damaged: {e}"))?;
    let names: Vec<PathBuf> = (0..archive.len()).filter_map(|i| archive.by_index(i).ok().and_then(|f| f.enclosed_name())).collect();
    let top = names.first().and_then(|p| p.components().next()).map(|c| c.as_os_str().to_owned());
    let strip = top.is_some() && names.iter().all(|p| p.components().next().map(|c| Some(c.as_os_str().to_owned()) == top).unwrap_or(false)) && names.iter().any(|p| p.components().count() > 1);
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let total = archive.len();
    for i in 0..total {
        let mut file = archive.by_index(i).map_err(|e| e.to_string())?;
        // enclosed_name rejects absolute paths and ".." — nothing can be written outside `dir`.
        let Some(name) = file.enclosed_name() else { continue };
        let rel: PathBuf = if strip { name.components().skip(1).collect() } else { name };
        if rel.as_os_str().is_empty() { continue; }
        let target = dir.join(&rel);
        if file.is_dir() { std::fs::create_dir_all(&target).map_err(|e| e.to_string())?; continue; }
        if file.is_symlink() { continue; }
        if let Some(parent) = target.parent() { std::fs::create_dir_all(parent).map_err(|e| e.to_string())?; }
        let mut data = Vec::with_capacity(file.size() as usize); file.read_to_end(&mut data).map_err(|e| e.to_string())?;
        if std::fs::write(&target, &data).is_err() {
            // A running executable cannot be written (ETXTBSY), but it can be renamed.
            let old = target.with_extension(format!("{}.old", target.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default()));
            let _ = std::fs::remove_file(&old);
            std::fs::rename(&target, &old).map_err(|e| crate::trf!("{} занят: {e}", "{} is in use: {e}", rel.display()))?;
            std::fs::write(&target, &data).map_err(|e| format!("{}: {e}", rel.display()))?;
        }
        // Executable bits from the archive (zip from Linux), and always for the two programs.
        let base = rel.file_name().and_then(|n| n.to_str()).unwrap_or("");
        let exec = file.unix_mode().map(|m| m & 0o111 != 0).unwrap_or(false) || base == "wraith" || base == "xray";
        let _ = std::fs::set_permissions(&target, std::fs::Permissions::from_mode(if exec { 0o755 } else { 0o644 }));
        progress(i + 1, total);
    }
    Ok(())
}

/// Leftovers of an update that replaced files in use.
pub fn clean_old_files() {
    if let Ok(entries) = std::fs::read_dir(util::app_dir()) {
        for e in entries.flatten() { if e.path().extension().map(|x| x == "old").unwrap_or(false) { let _ = std::fs::remove_file(e.path()); } }
    }
}

/// Wraith and Xray processes of this user started from `dir` (except this process).
pub fn kill_in_dir(dir: &Path) {
    let me = std::process::id();
    for (pid, exe) in util::processes(&["wraith", "xray", "sing-box"]) {
        if pid != me && exe.starts_with(dir) && util::process_uid(pid) == Some(util::my_uid()) { util::terminate(pid, 3000); }
    }
}

// ------------------------------------------------------------------ desktop entries

fn desktop_entry(exe: &Path, autostart: bool) -> String {
    let exe = exe.to_string_lossy().replace('"', "");
    let mut s = format!("[Desktop Entry]\nType=Application\nName=Wraith\nGenericName=VPN client\nGenericName[ru]=VPN-клиент\nComment=Xray VPN client\nComment[ru]=VPN-клиент на ядре Xray\nExec=\"{exe}\"{}\nIcon=wraith\nTerminal=false\nCategories=Network;\nKeywords=vpn;xray;vless;proxy;\nStartupWMClass=wraith\n", if autostart { " --tray" } else { "" });
    if autostart { s += "X-GNOME-Autostart-enabled=true\nX-KDE-autostart-after=panel\n"; }
    else { s += &format!("Actions=uninstall;\n\n[Desktop Action uninstall]\nName=Uninstall Wraith\nName[ru]=Удалить Wraith\nExec=\"{exe}\" --uninstall\n"); }
    s
}
fn autostart_file() -> PathBuf { util::config_home().join("autostart/wraith.desktop") }
fn desktop_dir() -> PathBuf {
    util::output("xdg-user-dir", &["DESKTOP"]).map(PathBuf::from).filter(|p| p.is_absolute() && p != &util::home()).unwrap_or_else(|| util::home().join("Desktop"))
}
fn desktop_shortcut() -> PathBuf { desktop_dir().join("wraith.desktop") }
fn icon_png() -> Vec<u8> {
    let mut out = vec![];
    { let mut enc = png::Encoder::new(&mut out, 256, 256); enc.set_color(png::ColorType::Rgba); enc.set_depth(png::BitDepth::Eight);
      if let Ok(mut w) = enc.write_header() { let _ = w.write_image_data(&crate::ui::icon::rgba(256)); } }
    out
}

/// Autostart at logon (XDG autostart entry), minimised to the tray.
pub fn set_autostart(on: bool) -> Result<(), String> {
    let file = autostart_file();
    if on {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        if let Some(parent) = file.parent() { std::fs::create_dir_all(parent).map_err(|e| e.to_string())?; }
        std::fs::write(&file, desktop_entry(&exe, true)).map_err(|e| crate::trf!("Не удалось включить автозапуск: {e}", "Could not turn on autostart: {e}"))
    } else { let _ = std::fs::remove_file(file); Ok(()) }
}

fn write_shortcut(exe: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let path = desktop_shortcut();
    if std::fs::create_dir_all(desktop_dir()).is_err() || std::fs::write(&path, desktop_entry(exe, false)).is_err() { return; }
    let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755));
    // GNOME and KDE start a desktop file only once it is marked trusted.
    if util::which("gio").is_some() { util::run_quiet("gio", &["set", &path.to_string_lossy(), "metadata::trusted", "true"]); }
}

// ------------------------------------------------------------------ root step

/// Copies a staged tree into `dir` as root (atomically replacing the old folder), gives the core its
/// network capabilities and installs the system menu entry and icon.
const DEPLOY: &str = r#"set -eu
stage="$1"; dir="$2"
[ -x "$stage/app/wraith" ] || { echo "no wraith in the archive" >&2; exit 3; }
case "$dir" in /opt/*) ;; *) echo "refusing $dir" >&2; exit 4;; esac
rm -rf "$dir.new"; mkdir -p "$dir.new"
cp -R "$stage/app/." "$dir.new/"
chown -R root:root "$dir.new"; chmod -R u+rwX,go+rX,go-w "$dir.new"
if [ -e "$dir" ]; then rm -rf "$dir.old"; mv "$dir" "$dir.old"; fi
mv "$dir.new" "$dir"; rm -rf "$dir.old"
if [ -f "$dir/xray" ]; then setcap cap_net_admin,cap_net_raw,cap_net_bind_service+ep "$dir/xray"; fi
install -Dm644 "$stage/wraith.desktop" /usr/share/applications/wraith.desktop
install -Dm644 "$stage/wraith.png" /usr/share/icons/hicolor/256x256/apps/wraith.png
mkdir -p /usr/local/bin; ln -sfn "$dir/wraith" /usr/local/bin/wraith
command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database -q /usr/share/applications || true
command -v gtk-update-icon-cache >/dev/null 2>&1 && gtk-update-icon-cache -qtf /usr/share/icons/hicolor || true
"#;

fn stage_dir() -> Result<PathBuf, String> {
    let base = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).filter(|p| p.is_dir()).unwrap_or_else(std::env::temp_dir);
    let dir = base.join(format!("wraith-stage-{}", util::random_hex(6)));
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    { use std::os::unix::fs::PermissionsExt; let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)); }
    Ok(dir)
}

fn deploy(zip: &[u8], dir: &Path) -> Result<(), String> {
    let stage = stage_dir()?;
    let result = (|| {
        extract(zip, &stage.join("app"), &|_, _| {})?;
        std::fs::write(stage.join("app/wraith.installed"), env!("CARGO_PKG_VERSION")).map_err(|e| e.to_string())?;
        std::fs::write(stage.join("wraith.desktop"), desktop_entry(&dir.join("wraith"), false)).map_err(|e| e.to_string())?;
        std::fs::write(stage.join("wraith.png"), icon_png()).map_err(|e| e.to_string())?;
        util::pkexec("/bin/sh", &["-c", DEPLOY, "wraith-deploy", &stage.to_string_lossy(), &dir.to_string_lossy()])
    })();
    let _ = std::fs::remove_dir_all(&stage);
    result
}

/// Update of the installed copy (called by the updater).
pub fn privileged_update(zip: &[u8], dir: &Path) -> Result<(), String> {
    if dir != install_dir() { return Err(crate::trf!("Неожиданная папка установки: {}", "Unexpected install folder: {}", dir.display())); }
    deploy(zip, dir)
}

pub struct Options { pub autostart: bool, pub desktop: bool }

/// Full installation (the setup runs as the user; the root part goes through pkexec).
pub fn install(payload: &[u8], options: &Options, status: &dyn Fn(&str)) -> Result<PathBuf, String> {
    let dir = install_dir();
    status(crate::tr!("Останавливаем запущенный Wraith…", "Stopping the running Wraith…"));
    kill_in_dir(&dir);
    crate::core::proxy::restore_leftover();
    status(crate::tr!("Копируем файлы (нужен пароль администратора)…", "Copying files (the administrator password is needed)…"));
    deploy(payload, &dir)?;
    let exe = dir.join("wraith");
    status(crate::tr!("Настраиваем автозапуск…", "Setting up autostart…"));
    // The autostart entry points at the installed copy, not at this setup program.
    if options.autostart {
        let file = autostart_file();
        if let Some(parent) = file.parent() { let _ = std::fs::create_dir_all(parent); }
        std::fs::write(&file, desktop_entry(&exe, true)).map_err(|e| e.to_string())?;
    } else { let _ = std::fs::remove_file(autostart_file()); }
    status(crate::tr!("Создаём ярлыки…", "Creating shortcuts…"));
    if options.desktop { write_shortcut(&exe); }
    crate::store::update(|s| s.set("autostart", options.autostart.into()));
    Ok(exe)
}

/// `wraith --uninstall` (from the menu entry's "Uninstall" action).
pub fn uninstall() {
    if util::packaged() { crate::ui::dialog::ask(crate::tr!(util::PACKAGE_HINT_RU, util::PACKAGE_HINT_EN), false); return; }
    if !crate::ui::dialog::ask(crate::tr!("Удалить Wraith с этого компьютера?", "Remove Wraith from this computer?"), true) { return; }
    let dir = util::app_dir();
    crate::store::init();
    crate::core::proxy::restore_leftover();
    kill_in_dir(&dir);
    let _ = set_autostart(false);
    if std::fs::read_to_string(desktop_shortcut()).map(|t| t.contains(&*dir.to_string_lossy())).unwrap_or(false) { let _ = std::fs::remove_file(desktop_shortcut()); }
    // Only the real installation folder is removed; a portable copy stays where the user put it.
    if util::installed() && dir == install_dir() {
        let script = format!(r#"set -u
rm -rf "$1"
rm -f {SYSTEM_DESKTOP} {SYSTEM_ICON}
[ "$(readlink {SYSTEM_LINK})" = "$1/wraith" ] && rm -f {SYSTEM_LINK}
command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database -q /usr/share/applications
exit 0"#);
        if let Err(e) = util::pkexec("/bin/sh", &["-c", &script, "wraith-uninstall", &dir.to_string_lossy()]) {
            crate::ui::dialog::ask(&crate::trf!("Не удалось удалить файлы: {e}", "Could not remove the files: {e}"), false);
            return;
        }
    }
    if crate::ui::dialog::ask(crate::tr!("Удалить и ваши серверы, подписки и настройки?\n\nНет — оставить их для повторной установки.", "Also remove your servers, subscriptions and settings?\n\nNo keeps them for a reinstall."), true) {
        let _ = std::fs::remove_dir_all(crate::store::folder());
    }
    crate::ui::dialog::ask(crate::tr!("Wraith удалён.", "Wraith has been removed."), false);
}
