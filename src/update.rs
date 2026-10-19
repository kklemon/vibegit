//! Best-effort check for a newer release on PyPI.

use std::cmp::Ordering;
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

const PYPI_URL: &str = "https://pypi.org/pypi/vibegit/json";

fn parts(version: &str) -> Vec<u64> {
    version
        .split('.')
        .map(|p| {
            let digits: String = p.chars().take_while(|c| c.is_ascii_digit()).collect();
            digits.parse().unwrap_or(0)
        })
        .collect()
}

/// Numeric comparison of dotted versions (`0.3.10` > `0.3.9`); suffixes are ignored.
pub fn compare_versions(a: &str, b: &str) -> Ordering {
    let (a, b) = (parts(a), parts(b));
    let len = a.len().max(b.len());
    for i in 0..len {
        let x = a.get(i).copied().unwrap_or(0);
        let y = b.get(i).copied().unwrap_or(0);
        match x.cmp(&y) {
            Ordering::Equal => continue,
            other => return other,
        }
    }
    Ordering::Equal
}

/// Returns the latest published version when it is newer than `current`.
pub fn check_for_update(current: &str) -> Option<String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(3))
        .user_agent(format!("vibegit/{current}"))
        .build()
        .ok()?;
    let body: serde_json::Value = client.get(PYPI_URL).send().ok()?.json().ok()?;
    let latest = body.get("info")?.get("version")?.as_str()?.to_string();
    if compare_versions(&latest, current) == Ordering::Greater {
        Some(latest)
    } else {
        None
    }
}

/// Runs the check on a background thread; the receiver yields at most one message.
pub fn spawn_update_check(current: &'static str) -> Receiver<Option<String>> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(check_for_update(current));
    });
    rx
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_numerically() {
        assert_eq!(compare_versions("0.3.10", "0.3.9"), Ordering::Greater);
        assert_eq!(compare_versions("0.3", "0.3.0"), Ordering::Equal);
        assert_eq!(compare_versions("1.0.0rc1", "1.0.0"), Ordering::Equal);
        assert_eq!(compare_versions("0.2.2", "0.3.0"), Ordering::Less);
    }
}
