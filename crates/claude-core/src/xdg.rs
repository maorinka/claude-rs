//! XDG Base Directory resolver.
//!
//! Port of TS `src/utils/xdg.ts`. Used by the native-installer and
//! any persistent state / cache / data dirs to honour the XDG Base
//! Directory spec when `XDG_STATE_HOME` / `XDG_CACHE_HOME` /
//! `XDG_DATA_HOME` are set, and fall back to the conventional
//! `~/.local/state`, `~/.cache`, `~/.local/share` otherwise.
//!
//! Each helper accepts an `XdgOptions` override so tests can inject
//! a fake environment and homedir without touching process state.
//!
//! Note: `get_user_bin_dir` is not technically an XDG path but
//! lives here for symmetry — `~/.local/bin` is where the installer
//! writes the claude shim.
//!
//! <https://specifications.freedesktop.org/basedir-spec/latest/>

use std::collections::HashMap;
use std::path::PathBuf;

/// Overrides for env + homedir. Both are optional and fall back to
/// `std::env::var` + `dirs::home_dir` when absent.
#[derive(Default, Clone)]
pub struct XdgOptions {
    /// Map of env vars. When `None`, `std::env::var` is consulted.
    pub env: Option<HashMap<String, String>>,
    /// Home dir override. When `None`, `HOME` env var then
    /// `dirs::home_dir()` are tried.
    pub homedir: Option<PathBuf>,
}

fn lookup_env(options: &XdgOptions, key: &str) -> Option<String> {
    if let Some(map) = &options.env {
        return map.get(key).cloned();
    }
    std::env::var(key).ok()
}

fn resolve_home(options: &XdgOptions) -> PathBuf {
    options
        .homedir
        .clone()
        .or_else(|| lookup_env(options, "HOME").map(PathBuf::from))
        .or_else(dirs::home_dir)
        .unwrap_or_else(|| PathBuf::from("/"))
}

/// `$XDG_STATE_HOME` or `~/.local/state`.
pub fn get_xdg_state_home(options: &XdgOptions) -> PathBuf {
    lookup_env(options, "XDG_STATE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| resolve_home(options).join(".local").join("state"))
}

/// `$XDG_CACHE_HOME` or `~/.cache`.
pub fn get_xdg_cache_home(options: &XdgOptions) -> PathBuf {
    lookup_env(options, "XDG_CACHE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| resolve_home(options).join(".cache"))
}

/// `$XDG_DATA_HOME` or `~/.local/share`.
pub fn get_xdg_data_home(options: &XdgOptions) -> PathBuf {
    lookup_env(options, "XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| resolve_home(options).join(".local").join("share"))
}

/// `~/.local/bin` — not strictly XDG but follows the convention.
pub fn get_user_bin_dir(options: &XdgOptions) -> PathBuf {
    resolve_home(options).join(".local").join("bin")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk_env(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn opts(env: HashMap<String, String>, home: &str) -> XdgOptions {
        XdgOptions {
            env: Some(env),
            homedir: Some(PathBuf::from(home)),
        }
    }

    #[test]
    fn state_home_uses_env_when_set() {
        let o = opts(mk_env(&[("XDG_STATE_HOME", "/tmp/state")]), "/home/alex");
        assert_eq!(get_xdg_state_home(&o), PathBuf::from("/tmp/state"));
    }

    #[test]
    fn state_home_falls_back_to_default() {
        let o = opts(mk_env(&[]), "/home/alex");
        assert_eq!(
            get_xdg_state_home(&o),
            PathBuf::from("/home/alex/.local/state")
        );
    }

    #[test]
    fn cache_home_uses_env_when_set() {
        let o = opts(mk_env(&[("XDG_CACHE_HOME", "/tmp/cache")]), "/home/alex");
        assert_eq!(get_xdg_cache_home(&o), PathBuf::from("/tmp/cache"));
    }

    #[test]
    fn cache_home_falls_back_to_default() {
        let o = opts(mk_env(&[]), "/home/alex");
        assert_eq!(get_xdg_cache_home(&o), PathBuf::from("/home/alex/.cache"));
    }

    #[test]
    fn data_home_uses_env_when_set() {
        let o = opts(mk_env(&[("XDG_DATA_HOME", "/tmp/data")]), "/home/alex");
        assert_eq!(get_xdg_data_home(&o), PathBuf::from("/tmp/data"));
    }

    #[test]
    fn data_home_falls_back_to_default() {
        let o = opts(mk_env(&[]), "/home/alex");
        assert_eq!(
            get_xdg_data_home(&o),
            PathBuf::from("/home/alex/.local/share")
        );
    }

    #[test]
    fn user_bin_dir_ignores_env() {
        let o = opts(mk_env(&[("XDG_STATE_HOME", "/x")]), "/home/u");
        assert_eq!(get_user_bin_dir(&o), PathBuf::from("/home/u/.local/bin"));
    }

    #[test]
    fn home_from_env_when_no_homedir_override() {
        let o = XdgOptions {
            env: Some(mk_env(&[("HOME", "/envhome")])),
            homedir: None,
        };
        assert_eq!(
            get_xdg_state_home(&o),
            PathBuf::from("/envhome/.local/state")
        );
    }
}
