//! Cross-platform HOME / Desktop / Documents / Downloads resolver.
//!
//! Port of TS `utils/systemDirectories.ts:1-75`.
//!
//! Used for "save to…" dialog defaults, WebFetch downloads, and file-
//! picker seed paths. Platform differences handled:
//! - **Windows**: honours `USERPROFILE` so localised folder names
//!   (`Bureau`, `Escritorio`, …) and non-standard home dirs resolve.
//! - **Linux / WSL**: honours the XDG user-dirs env vars
//!   (`XDG_DESKTOP_DIR` etc.) that systemd writes from
//!   `~/.config/user-dirs.dirs`.
//! - **macOS / fallback**: default `~/Desktop`, `~/Documents`,
//!   `~/Downloads`.

use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemDirectories {
    pub home: PathBuf,
    pub desktop: PathBuf,
    pub documents: PathBuf,
    pub downloads: PathBuf,
}

/// Platform enum matching TS `Platform` union (narrower — Rust handles
/// WSL detection via `/proc/version` like TS `getPlatform`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    Macos,
    Windows,
    Linux,
    Wsl,
    Unknown,
}

fn detect_platform() -> Platform {
    if cfg!(target_os = "macos") {
        Platform::Macos
    } else if cfg!(target_os = "windows") {
        Platform::Windows
    } else if cfg!(target_os = "linux") {
        // WSL detection: `/proc/sys/kernel/osrelease` contains
        // "microsoft" on WSL 1/2. Falls back to plain Linux if that
        // read fails.
        if std::fs::read_to_string("/proc/sys/kernel/osrelease")
            .map(|s| s.to_ascii_lowercase().contains("microsoft"))
            .unwrap_or(false)
        {
            Platform::Wsl
        } else {
            Platform::Linux
        }
    } else {
        Platform::Unknown
    }
}

/// Options for test injection. TS accepts the same three overrides
/// (`env`, `homedir`, `platform`) so unit tests can exercise each
/// platform branch without needing `#[cfg]`.
#[derive(Default)]
pub struct SystemDirectoriesOptions<'a> {
    pub home: Option<PathBuf>,
    pub platform: Option<Platform>,
    pub get_env: Option<&'a dyn Fn(&str) -> Option<String>>,
}

fn default_env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

/// Resolve the cross-platform system directories.
pub fn get_system_directories(opts: &SystemDirectoriesOptions<'_>) -> SystemDirectories {
    let platform = opts.platform.unwrap_or_else(detect_platform);
    let home = opts
        .home
        .clone()
        .or_else(dirs::home_dir)
        .unwrap_or_else(|| PathBuf::from("/"));

    let default_get = |n: &str| default_env(n);
    let get = opts
        .get_env
        .map(|f| f as &dyn Fn(&str) -> Option<String>)
        .unwrap_or(&default_get);

    let defaults = SystemDirectories {
        home: home.clone(),
        desktop: home.join("Desktop"),
        documents: home.join("Documents"),
        downloads: home.join("Downloads"),
    };

    match platform {
        Platform::Windows => {
            // TS uses `env.USERPROFILE || homeDir` — Rust mirrors the
            // same fallback order exactly.
            let user_profile = get("USERPROFILE")
                .map(PathBuf::from)
                .unwrap_or_else(|| home.clone());
            SystemDirectories {
                home,
                desktop: user_profile.join("Desktop"),
                documents: user_profile.join("Documents"),
                downloads: user_profile.join("Downloads"),
            }
        }
        Platform::Linux | Platform::Wsl => SystemDirectories {
            home: defaults.home,
            desktop: get("XDG_DESKTOP_DIR").map(PathBuf::from).unwrap_or(defaults.desktop),
            documents: get("XDG_DOCUMENTS_DIR")
                .map(PathBuf::from)
                .unwrap_or(defaults.documents),
            downloads: get("XDG_DOWNLOAD_DIR")
                .map(PathBuf::from)
                .unwrap_or(defaults.downloads),
        },
        Platform::Macos | Platform::Unknown => defaults,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_home() -> PathBuf {
        PathBuf::from("/home/testuser")
    }

    #[test]
    fn macos_returns_defaults() {
        let dirs = get_system_directories(&SystemDirectoriesOptions {
            home: Some(fake_home()),
            platform: Some(Platform::Macos),
            get_env: None,
        });
        assert_eq!(dirs.home, fake_home());
        assert_eq!(dirs.desktop, fake_home().join("Desktop"));
        assert_eq!(dirs.documents, fake_home().join("Documents"));
        assert_eq!(dirs.downloads, fake_home().join("Downloads"));
    }

    #[test]
    fn windows_uses_userprofile_for_folders() {
        let env = |n: &str| match n {
            "USERPROFILE" => Some("C:/Users/alice".to_string()),
            _ => None,
        };
        let dirs = get_system_directories(&SystemDirectoriesOptions {
            home: Some(PathBuf::from("C:/Users/alice")),
            platform: Some(Platform::Windows),
            get_env: Some(&env),
        });
        assert_eq!(dirs.desktop, PathBuf::from("C:/Users/alice/Desktop"));
        assert_eq!(dirs.documents, PathBuf::from("C:/Users/alice/Documents"));
    }

    #[test]
    fn windows_falls_back_to_home_when_userprofile_unset() {
        let env = |_: &str| None;
        let dirs = get_system_directories(&SystemDirectoriesOptions {
            home: Some(fake_home()),
            platform: Some(Platform::Windows),
            get_env: Some(&env),
        });
        assert_eq!(dirs.desktop, fake_home().join("Desktop"));
    }

    #[test]
    fn linux_honours_xdg_env_vars() {
        let env = |n: &str| match n {
            "XDG_DESKTOP_DIR" => Some("/home/testuser/Bureau".into()),
            "XDG_DOCUMENTS_DIR" => Some("/home/testuser/Documents".into()),
            "XDG_DOWNLOAD_DIR" => Some("/home/testuser/Téléchargements".into()),
            _ => None,
        };
        let dirs = get_system_directories(&SystemDirectoriesOptions {
            home: Some(fake_home()),
            platform: Some(Platform::Linux),
            get_env: Some(&env),
        });
        assert_eq!(dirs.desktop, PathBuf::from("/home/testuser/Bureau"));
        assert_eq!(
            dirs.downloads,
            PathBuf::from("/home/testuser/Téléchargements")
        );
    }

    #[test]
    fn linux_falls_back_when_xdg_unset() {
        let env = |_: &str| None;
        let dirs = get_system_directories(&SystemDirectoriesOptions {
            home: Some(fake_home()),
            platform: Some(Platform::Linux),
            get_env: Some(&env),
        });
        assert_eq!(dirs.desktop, fake_home().join("Desktop"));
    }

    #[test]
    fn wsl_behaves_like_linux_for_xdg() {
        let env = |n: &str| match n {
            "XDG_DOWNLOAD_DIR" => Some("/mnt/c/Users/alice/Downloads".into()),
            _ => None,
        };
        let dirs = get_system_directories(&SystemDirectoriesOptions {
            home: Some(fake_home()),
            platform: Some(Platform::Wsl),
            get_env: Some(&env),
        });
        assert_eq!(dirs.downloads, PathBuf::from("/mnt/c/Users/alice/Downloads"));
        // Defaults for unset XDG vars.
        assert_eq!(dirs.desktop, fake_home().join("Desktop"));
    }

    #[test]
    fn unknown_platform_returns_defaults() {
        let dirs = get_system_directories(&SystemDirectoriesOptions {
            home: Some(fake_home()),
            platform: Some(Platform::Unknown),
            get_env: None,
        });
        assert_eq!(dirs.desktop, fake_home().join("Desktop"));
    }
}
