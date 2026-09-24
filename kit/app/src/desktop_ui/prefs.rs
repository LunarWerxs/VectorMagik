//! Part of `desktop_ui`: the window's preferences, kept between runs in a
//! small text file of `name=on|off` lines. Only the window reads and writes
//! it; snapshots and tests start from the defaults every time.

use super::*;

/// `%APPDATA%\VectorMagik\desktop.txt`, or the XDG config folder elsewhere.
pub(super) fn prefs_path() -> Option<PathBuf> {
    let set = |name: &str| std::env::var_os(name).filter(|v| !v.is_empty());
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

pub(super) fn write_prefs((hold, auto): (bool, bool)) -> String {
    let word = |on: bool| if on { "on" } else { "off" };
    format!(
        "hold_compare={}\r\nauto_convert={}\r\n",
        word(hold),
        word(auto)
    )
}

impl Desktop {
    /// Take the preferences the window was left with from `path`; a missing
    /// or unreadable file keeps the defaults.
    pub(super) fn load_prefs(&mut self, path: PathBuf) {
        if let Ok(text) = std::fs::read_to_string(&path) {
            (self.hold_compare, self.auto_convert) =
                parse_prefs(&text, (self.hold_compare, self.auto_convert));
        }
        self.prefs_saved = (self.hold_compare, self.auto_convert);
        self.prefs = Some(path);
    }
    /// Write the preferences when one has changed; a failure costs only the
    /// memory of them, so it is not reported.
    pub(super) fn save_prefs(&mut self) {
        let now = (self.hold_compare, self.auto_convert);
        let Some(path) = &self.prefs else {
            return;
        };
        if now == self.prefs_saved {
            return;
        }
        self.prefs_saved = now;
        if let Some(folder) = path.parent() {
            let _ = std::fs::create_dir_all(folder);
        }
        let _ = std::fs::write(path, write_prefs(now));
    }
}
