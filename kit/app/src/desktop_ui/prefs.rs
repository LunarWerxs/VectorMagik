//! Part of `desktop_ui`: the window's preferences, kept between runs in a
//! small text file of `name=on|off` lines, the look (`look=`, `theme=`), the
//! answer to the first-run question (`licence_use=`) and the commercial
//! licence's key and certificate (`licence_key=`, `licence_certificate=`).
//! The file sits beside the program when its folder takes files, so the app
//! is portable (`prefs_path`). Only the window reads and writes it (a tab
//! keeps the same text in the page's storage); snapshots and tests start
//! from the defaults every time.

use super::*;

/// The settings file of a portable copy, beside the program.
#[cfg(feature = "desktop")]
pub(super) const PORTABLE_PREFS: &str = "VectorMagik-settings.txt";

/// Where the window keeps its preferences: beside the program when its
/// folder takes files, so a copy on a USB stick carries its settings and
/// licence and leaves nothing behind on the computer; else the installed
/// place (`installed_prefs`), for a copy in a folder it may not write.
#[cfg(feature = "desktop")]
pub(super) fn prefs_path() -> Option<PathBuf> {
    // A Mac app is a signed bundle: nothing is written inside it.
    if cfg!(target_os = "macos") {
        return installed_prefs();
    }
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().and_then(portable_prefs))
        .or_else(installed_prefs)
}

/// The settings file in `folder` when it is already there or the folder
/// takes a new file.
#[cfg(feature = "desktop")]
pub(super) fn portable_prefs(folder: &Path) -> Option<PathBuf> {
    let path = folder.join(PORTABLE_PREFS);
    if path.is_file() {
        return Some(path);
    }
    let probe = folder.join(".vectormagik-write-test");
    let writable = std::fs::write(&probe, b"").is_ok();
    let _ = std::fs::remove_file(&probe);
    writable.then_some(path)
}

/// `%APPDATA%\VectorMagik\desktop.txt`, `~/Library/Application Support` on a
/// Mac, or the XDG config folder elsewhere: where the settings lived before
/// the app kept them beside itself.
#[cfg(feature = "desktop")]
pub(super) fn installed_prefs() -> Option<PathBuf> {
    let set = |name: &str| std::env::var_os(name).filter(|v| !v.is_empty());
    if cfg!(target_os = "macos") {
        let home = PathBuf::from(set("HOME")?);
        return Some(
            home.join("Library/Application Support")
                .join("VectorMagik")
                .join("desktop.txt"),
        );
    }
    let base = set("APPDATA")
        .map(PathBuf::from)
        .or_else(|| set("XDG_CONFIG_HOME").map(PathBuf::from))
        .or_else(|| set("HOME").map(|home| PathBuf::from(home).join(".config")))?;
    Some(base.join("VectorMagik").join("desktop.txt"))
}

/// The preferences in `text`, over `defaults` (hold to compare, convert
/// automatically); lines it does not know are ignored.
pub(super) fn parse_prefs(text: &str, defaults: (bool, bool)) -> (bool, bool) {
    let (mut hold, mut auto) = defaults;
    for line in text.lines() {
        let Some((name, value)) = line.split_once('=') else {
            continue;
        };
        let on = match value.trim() {
            "on" => true,
            "off" => false,
            _ => continue,
        };
        match name.trim() {
            "hold_compare" => hold = on,
            "auto_convert" => auto = on,
            _ => {}
        }
    }
    (hold, auto)
}

/// The licence kept in `text`: its key (only a well-shaped one) and its
/// certificate.
pub(super) fn parse_licence(text: &str) -> crate::licence::Stored {
    let mut stored = crate::licence::Stored::default();
    for line in text.lines() {
        match line.split_once('=') {
            Some(("licence_key", key)) => {
                stored.key = crate::licence::normalize_key(key).unwrap_or_default();
            }
            Some(("licence_certificate", certificate)) => {
                stored.certificate = certificate.trim().to_owned();
            }
            _ => {}
        }
    }
    if stored.key.is_empty() {
        stored.certificate.clear();
    }
    stored
}

/// The look, light or dark and the first-run answer kept in `text`; a line
/// missing or not understood keeps Classic, dark, not yet asked.
pub(super) fn parse_appearance(text: &str) -> (Look, ThemeChoice, Option<LicenceUse>) {
    let (mut look, mut theme, mut answer) = (Look::Classic, ThemeChoice::Dark, None);
    for line in text.lines() {
        match line.split_once('=') {
            Some(("look", word)) => look = Look::from_word(word).unwrap_or(look),
            Some(("theme", word)) => theme = ThemeChoice::from_word(word).unwrap_or(theme),
            Some(("licence_use", word)) => answer = LicenceUse::from_word(word),
            _ => {}
        }
    }
    (look, theme, answer)
}

pub(super) fn write_prefs(
    (hold, auto): (bool, bool),
    licence: &crate::licence::Stored,
    (look, theme, answer): (Look, ThemeChoice, Option<LicenceUse>),
) -> String {
    let word = |on: bool| if on { "on" } else { "off" };
    let mut text = format!(
        "hold_compare={}\r\nauto_convert={}\r\nlook={}\r\ntheme={}\r\n",
        word(hold),
        word(auto),
        look.word(),
        theme.word()
    );
    if let Some(answer) = answer {
        text.push_str(&format!("licence_use={}\r\n", answer.word()));
    }
    if !licence.key.is_empty() {
        text.push_str(&format!(
            "licence_key={}\r\nlicence_certificate={}\r\n",
            licence.key, licence.certificate
        ));
    }
    text
}

impl Desktop {
    /// Take the preferences the window was left with from `path`; a missing
    /// or unreadable file keeps the defaults.
    #[cfg(feature = "desktop")]
    pub(super) fn load_prefs(&mut self, path: PathBuf) {
        // A portable copy's first run carries over what the installed place
        // kept, the licence included.
        let portable = path.file_name().is_some_and(|name| name == PORTABLE_PREFS);
        let text = std::fs::read_to_string(&path).ok().or_else(|| {
            installed_prefs()
                .filter(|old| portable && *old != path)
                .and_then(|old| std::fs::read_to_string(old).ok())
        });
        if let Some(text) = text {
            (self.hold_compare, self.auto_convert) =
                parse_prefs(&text, (self.hold_compare, self.auto_convert));
            self.licence = parse_licence(&text);
            (self.look, self.theme, self.licence_use) = parse_appearance(&text);
        }
        self.prefs_saved = (self.hold_compare, self.auto_convert);
        self.licence_saved = self.licence.clone();
        self.appearance_saved = (self.look, self.theme, self.licence_use);
        self.prefs = Some(path);
    }
    /// Write the preferences when one has changed; a failure costs only the
    /// memory of them, so it is not reported.
    pub(super) fn save_prefs(&mut self) {
        let now = (self.hold_compare, self.auto_convert);
        let appearance = (self.look, self.theme, self.licence_use);
        let changed = now != self.prefs_saved
            || self.licence != self.licence_saved
            || appearance != self.appearance_saved;
        if platform::IN_BROWSER && changed {
            self.prefs_saved = now;
            self.licence_saved = self.licence.clone();
            self.appearance_saved = appearance;
            platform::ask(platform::Command::StorePrefs(write_prefs(
                now,
                &self.licence,
                appearance,
            )));
            return;
        }
        let Some(path) = &self.prefs else {
            return;
        };
        if !changed {
            return;
        }
        self.prefs_saved = now;
        self.licence_saved = self.licence.clone();
        self.appearance_saved = appearance;
        if let Some(folder) = path.parent() {
            let _ = std::fs::create_dir_all(folder);
        }
        let _ = std::fs::write(path, write_prefs(now, &self.licence, appearance));
    }
}
