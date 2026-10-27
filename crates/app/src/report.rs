//! "Сообщить о проблеме": a new issue on GitHub from the bug form, with the version, the system
//! and the end of the log already filled in. The user sees it all before sending.

use crate::core::cache_dir;
use crate::updates::CURRENT;

const NEW_ISSUE: &str = "https://github.com/onokashino/ninari/issues/new";
/// GitHub refuses addresses much past 8 KB; the log is cut to fit.
const LOG_MAX: usize = 5000;

/// The system as a person would name it: "CachyOS · Wayland · niri".
fn system() -> String {
    if cfg!(windows) {
        return "Windows".into();
    }
    let name = std::fs::read_to_string("/etc/os-release")
        .ok()
        .and_then(|t| {
            t.lines()
                .find_map(|l| l.strip_prefix("PRETTY_NAME="))
                .map(|v| v.trim_matches('"').to_string())
        })
        .unwrap_or_else(|| std::env::consts::OS.into());
    [
        Some(name),
        std::env::var("XDG_SESSION_TYPE").ok(),
        std::env::var("XDG_CURRENT_DESKTOP").ok(),
    ]
    .into_iter()
    .flatten()
    .filter(|s| !s.is_empty())
    .collect::<Vec<_>>()
    .join(" · ")
}

/// The last lines of `log` that fit, with the home folder written as ~ so no user name leaves.
fn tail(log: &str, home: &str) -> String {
    let log = if home.is_empty() {
        log.to_string()
    } else {
        log.replace(home, "~")
    };
    let mut out: Vec<&str> = Vec::new();
    let mut size = 0;
    for line in log.lines().rev() {
        size += line.len() + 1;
        if size > LOG_MAX {
            break;
        }
        out.push(line);
    }
    out.reverse();
    out.join("\n")
}

fn url(system: &str, log: &str) -> String {
    let mut url = url::Url::parse(NEW_ISSUE).expect("static url");
    url.query_pairs_mut()
        .append_pair("template", "bug.yml")
        .append_pair("version", CURRENT)
        .append_pair("system", system)
        .append_pair("log", log);
    url.into()
}

pub fn issue_url() -> String {
    let log = std::fs::read_to_string(cache_dir().join("ninari.log")).unwrap_or_default();
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_default();
    url(&system(), &tail(&log, &home))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_log_is_cut_from_the_start_and_hides_the_home_folder() {
        let log = (0..2000)
            .map(|i| format!("line {i} /home/alice/.cache/x"))
            .collect::<Vec<_>>()
            .join("\n");
        let t = tail(&log, "/home/alice");
        assert!(t.len() <= LOG_MAX);
        assert!(t.ends_with("line 1999 ~/.cache/x"));
        assert!(!t.contains("alice"));
        let u = url("Arch · Wayland", &t);
        assert!(u.starts_with(
            "https://github.com/onokashino/ninari/issues/new?template=bug.yml&version="
        ));
        assert!(u.len() < 8000 * 2, "{}", u.len());
    }
}
