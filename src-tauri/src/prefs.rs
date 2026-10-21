//! App preferences in `prefs.json`, kept apart from the SSH config.
//!
//! Users may edit the file by hand (e.g. to set `editor`), so unknown keys are
//! kept on save, and a file that does not parse is reported and never
//! overwritten.

use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Prefs {
    /// Custom SSH config, passed as `-F`; absent means the default config.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config_path: Option<PathBuf>,
    /// Size and collapsed state of the side bar and its views.
    #[serde(skip_serializing_if = "Layout::is_default")]
    pub layout: Layout,
    /// Editor for "Open config", run as `editor <file>`. Set by hand.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub editor: Option<PathBuf>,
    /// Host aliases opened in the app, most recent first.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub recent_hosts: Vec<String>,
    #[serde(flatten)]
    pub other: serde_json::Map<String, serde_json::Value>,
}

/// The side bar (hosts above files, right of the terminal), in pixels.
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Layout {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sidebar_width: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub files_height: Option<u32>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub sidebar_hidden: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub hosts_collapsed: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub files_collapsed: bool,
}

impl Layout {
    fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

/// How many recently opened hosts are remembered.
pub const RECENT_HOSTS: usize = 50;

impl Prefs {
    /// Moves `alias` to the front of the recent hosts.
    pub fn record_host(&mut self, alias: &str) {
        self.recent_hosts.retain(|a| a != alias);
        self.recent_hosts.insert(0, alias.to_string());
        self.recent_hosts.truncate(RECENT_HOSTS);
    }
}

/// Reads `path`; a missing file gives the defaults.
/// The settings folder's name before the app was renamed to Simter.
pub const LEGACY_DIR: &str = "dev.sshworkspace.desktop";

/// Copies preferences from `old` when there are none at `path` yet. The old
/// file is left alone.
pub fn adopt(old: &Path, path: &Path) -> Result<(), String> {
    if path.exists() || !old.exists() {
        return Ok(());
    }
    let dir = path.parent().ok_or("preferences path has no folder")?;
    fs::create_dir_all(dir).map_err(|e| format!("could not create {}: {e}", dir.display()))?;
    fs::copy(old, path)
        .map(|_| ())
        .map_err(|e| format!("could not copy {}: {e}", old.display()))
}

pub fn load(path: &Path) -> Result<Prefs, String> {
    match fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text)
            .map_err(|e| format!("{} is not valid preferences: {e}", path.display())),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(Prefs::default()),
        Err(e) => Err(format!("could not read {}: {e}", path.display())),
    }
}

/// Applies `change` to the stored preferences and writes them back. The file
/// is replaced atomically, so a crash never leaves it half written.
pub fn update(path: &Path, change: impl FnOnce(&mut Prefs)) -> Result<(), String> {
    let mut prefs = load(path)?;
    change(&mut prefs);
    let dir = path.parent().ok_or("preferences path has no folder")?;
    fs::create_dir_all(dir).map_err(|e| format!("could not create {}: {e}", dir.display()))?;
    let text = serde_json::to_string_pretty(&prefs).expect("Prefs serializes");
    let temp = temp_path(path);
    fs::write(&temp, text + "\n")
        .and_then(|()| fs::rename(&temp, path))
        .map_err(|e| {
            let _ = fs::remove_file(&temp);
            format!("could not save {}: {e}", path.display())
        })
}

fn temp_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_owned();
    name.push(".tmp");
    path.with_file_name(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn adopts_old_preferences_once() {
        let dir = TempDir::new().unwrap();
        let old = dir.path().join("old").join("prefs.json");
        let path = dir.path().join("new").join("prefs.json");
        adopt(&old, &path).unwrap();
        assert!(!path.exists());
        fs::create_dir_all(old.parent().unwrap()).unwrap();
        fs::write(&old, r#"{"recentHosts":["a"]}"#).unwrap();
        adopt(&old, &path).unwrap();
        assert_eq!(load(&path).unwrap().recent_hosts, ["a"]);
        // Never overwrites settings made since.
        fs::write(&old, r#"{"recentHosts":["b"]}"#).unwrap();
        adopt(&old, &path).unwrap();
        assert_eq!(load(&path).unwrap().recent_hosts, ["a"]);
        assert!(old.exists());
    }

    #[test]
    fn missing_file_gives_defaults() {
        let dir = TempDir::new().unwrap();
        assert_eq!(
            load(&dir.path().join("prefs.json")).unwrap(),
            Prefs::default()
        );
    }

    #[test]
    fn round_trips_and_creates_the_folder() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("app data").join("prefs.json");
        update(&path, |p| {
            p.config_path = Some(r"D:\máy chủ\ssh config".into());
            p.layout.sidebar_width = Some(420);
        })
        .unwrap();
        let prefs = load(&path).unwrap();
        assert_eq!(prefs.config_path, Some(r"D:\máy chủ\ssh config".into()));
        assert_eq!(prefs.layout.sidebar_width, Some(420));
        assert!(!temp_path(&path).exists());
    }

    #[test]
    fn keeps_hand_edited_and_unknown_keys() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("prefs.json");
        fs::write(
            &path,
            r#"{ "editor": "C:\\Tools\\edit.exe", "future": [1, 2] }"#,
        )
        .unwrap();
        update(&path, |p| p.layout.sidebar_width = Some(300)).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value["editor"], r"C:\Tools\edit.exe");
        assert_eq!(value["future"], serde_json::json!([1, 2]));
        assert_eq!(value["layout"], serde_json::json!({ "sidebarWidth": 300 }));
    }

    #[test]
    fn clearing_the_config_path_removes_the_key() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("prefs.json");
        update(&path, |p| p.config_path = Some(r"D:\c".into())).unwrap();
        update(&path, |p| p.config_path = None).unwrap();
        assert!(!fs::read_to_string(&path).unwrap().contains("configPath"));
    }

    #[test]
    fn layout_keeps_only_what_differs_from_the_defaults() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("prefs.json");
        update(&path, |p| p.layout.files_collapsed = true).unwrap();
        let value: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(
            value["layout"],
            serde_json::json!({ "filesCollapsed": true })
        );
        update(&path, |p| p.layout = Layout::default()).unwrap();
        assert!(!fs::read_to_string(&path).unwrap().contains("layout"));
    }

    #[test]
    fn recent_hosts_move_to_the_front_without_duplicates() {
        let mut prefs = Prefs::default();
        for alias in ["a", "b", "c", "a"] {
            prefs.record_host(alias);
        }
        assert_eq!(prefs.recent_hosts, ["a", "c", "b"]);
        for i in 0..RECENT_HOSTS {
            prefs.record_host(&format!("h{i}"));
        }
        assert_eq!(prefs.recent_hosts.len(), RECENT_HOSTS);
        assert_eq!(prefs.recent_hosts[0], format!("h{}", RECENT_HOSTS - 1));
    }

    #[test]
    fn no_recent_hosts_key_until_one_is_opened() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("prefs.json");
        update(&path, |p| p.layout.sidebar_width = Some(1)).unwrap();
        assert!(!fs::read_to_string(&path).unwrap().contains("recentHosts"));
        update(&path, |p| p.record_host("web prod")).unwrap();
        assert_eq!(load(&path).unwrap().recent_hosts, ["web prod"]);
    }

    #[test]
    fn invalid_file_is_reported_and_left_alone() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("prefs.json");
        fs::write(&path, "{ editor: oops").unwrap();
        assert!(load(&path).unwrap_err().contains("not valid"));
        assert!(update(&path, |p| p.layout.sidebar_width = Some(1)).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "{ editor: oops");
    }
}
