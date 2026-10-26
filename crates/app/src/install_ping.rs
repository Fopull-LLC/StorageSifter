//! The anonymous install ping (Fopull install-ping contract, `contracts/install-ping.md`
//! in the hub repo): at most once per UTC day, a background `POST` of a random
//! install id plus the app version, OS and CPU architecture to fopull.com, so
//! active installs can be counted without counting people.
//!
//! Opt-out is the `install_ping` setting or `FOPULL_NO_PING` (any non-empty
//! value other than `0`). While opted out no request is made and no id exists.
//! The id lives in `install-ping.json` beside the settings file, together with
//! the UTC date of the last accepted ping.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

const ENDPOINT: &str = "https://fopull.com/api/v1/installs/ping";
const PRODUCT: &str = "storage-sifter";
/// Where the notice and the Settings toggle link to.
pub const PRIVACY_URL: &str = "https://fopull.com/privacy#install-ping";
/// Waits before each attempt: the first try, then ~1 min and ~10 min retries
/// on a network error or 5xx.
const ATTEMPT_DELAYS: [Duration; 3] = [
    Duration::ZERO,
    Duration::from_secs(60),
    Duration::from_secs(600),
];

/// Set by `forget`, so a ping thread waiting to retry gives up.
static OPTED_OUT: AtomicBool = AtomicBool::new(false);

#[derive(Serialize, Deserialize)]
struct State {
    install_id: String,
    /// UTC date (`YYYY-MM-DD`) of the last ping answered with a 204.
    #[serde(default)]
    last_ping: Option<String>,
}

#[derive(Serialize)]
struct Body<'a> {
    install_id: &'a str,
    product: &'a str,
    version: &'a str,
    os: &'a str,
    arch: &'a str,
}

/// `FOPULL_NO_PING` overrides the setting for every Fopull app.
pub fn disabled_by_env() -> bool {
    matches!(std::env::var("FOPULL_NO_PING"), Ok(v) if !v.is_empty() && v != "0")
}

fn state_path() -> Option<PathBuf> {
    Some(crate::settings::config_dir()?.join("install-ping.json"))
}

/// Opting out deletes the install id, so a later opt-in counts as a new install.
pub fn forget() {
    OPTED_OUT.store(true, Ordering::Relaxed);
    if let Some(path) = state_path() {
        let _ = std::fs::remove_file(path);
    }
}

/// Send today's ping on a background thread, unless one already went out today.
/// Never blocks the caller and never reports anything to the user.
pub fn spawn() {
    OPTED_OUT.store(false, Ordering::Relaxed);
    let _ = std::thread::Builder::new()
        .name("install-ping".into())
        .spawn(run);
}

fn run() {
    let Some(path) = state_path() else {
        return;
    };
    let today = utc_date(SystemTime::now());
    let mut state = match std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str::<State>(&s).ok())
    {
        Some(state) => state,
        None => {
            let Some(install_id) = new_uuid_v4() else {
                return;
            };
            State {
                install_id,
                last_ping: None,
            }
        }
    };
    if state.last_ping.as_deref() == Some(today.as_str()) {
        return;
    }
    if !save(&path, &state) {
        return; // without a stored id, every launch would count as a new install
    }

    let body = Body {
        install_id: &state.install_id,
        product: PRODUCT,
        version: env!("CARGO_PKG_VERSION"),
        os: os_name(),
        arch: std::env::consts::ARCH,
    };
    let Ok(json) = serde_json::to_string(&body) else {
        return;
    };

    for delay in ATTEMPT_DELAYS {
        std::thread::sleep(delay);
        if OPTED_OUT.load(Ordering::Relaxed) {
            return;
        }
        match post(&json) {
            Some((204, _)) => {
                // Don't recreate the id file if the user opted out meanwhile.
                if !OPTED_OUT.load(Ordering::Relaxed) {
                    state.last_ping = Some(utc_date(SystemTime::now()));
                    save(&path, &state);
                }
                return;
            }
            Some((400, reply)) => {
                eprintln!("storagesifter: install ping rejected: {}", reply.trim());
                return;
            }
            Some((429, _)) => return,
            // 5xx, an unexpected status, or a network error: retry, then give up.
            _ => {}
        }
    }
}

fn save(path: &PathBuf, state: &State) -> bool {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    serde_json::to_string_pretty(state)
        .ok()
        .is_some_and(|json| std::fs::write(path, json).is_ok())
}

/// POST through `curl` with a 10 s timeout. Returns the HTTP status and the
/// reply body, or `None` if the request didn't complete.
fn post(json: &str) -> Option<(u16, String)> {
    let mut child = Command::new("curl")
        .args(["-sS", "--max-time", "10", "-X", "POST"])
        .args(["-H", "Content-Type: application/json"])
        .args(["--data-binary", "@-", "-w", "\n%{http_code}", ENDPOINT])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    child.stdin.take()?.write_all(json.as_bytes()).ok()?;
    let out = child.wait_with_output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let (reply, code) = text.rsplit_once('\n')?;
    let code: u16 = code.trim().parse().ok()?;
    (code != 0).then(|| (code, reply.to_owned()))
}

fn os_name() -> &'static str {
    match std::env::consts::OS {
        os @ ("linux" | "windows" | "macos") => os,
        _ => "other",
    }
}

/// A random UUID v4 from the OS's cryptographic source.
fn new_uuid_v4() -> Option<String> {
    use std::io::Read;
    let mut b = [0u8; 16];
    std::fs::File::open("/dev/urandom")
        .ok()?
        .read_exact(&mut b)
        .ok()?;
    Some(format_uuid_v4(b))
}

fn format_uuid_v4(mut b: [u8; 16]) -> String {
    b[6] = (b[6] & 0x0f) | 0x40; // version 4
    b[8] = (b[8] & 0x3f) | 0x80; // RFC 4122 variant
    let hex: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

/// The UTC calendar date of `t` as `YYYY-MM-DD`.
fn utc_date(t: SystemTime) -> String {
    let days = t
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() / 86_400)
        .unwrap_or(0) as i64;
    // Howard Hinnant's civil-from-days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utc_date_matches_known_days() {
        let at = |secs| utc_date(UNIX_EPOCH + Duration::from_secs(secs));
        assert_eq!(at(0), "1970-01-01");
        assert_eq!(at(951_782_400), "2000-02-29");
        assert_eq!(at(1_793_059_199), "2026-10-26"); // last second of the day
        assert_eq!(at(1_793_059_200), "2026-10-27");
    }

    #[test]
    fn uuid_has_v4_shape() {
        let id = format_uuid_v4([0xff; 16]);
        assert_eq!(id, "ffffffff-ffff-4fff-bfff-ffffffffffff");
        let id = new_uuid_v4().unwrap();
        assert_eq!(id.len(), 36);
        assert_eq!(&id[14..15], "4");
        assert!(matches!(&id[19..20], "8" | "9" | "a" | "b"));
    }

    #[test]
    fn body_matches_the_contract() {
        let body = Body {
            install_id: "3f2b9c4e-8a1d-4c6b-9e2f-7d5a1b0c3e48",
            product: PRODUCT,
            version: env!("CARGO_PKG_VERSION"),
            os: os_name(),
            arch: std::env::consts::ARCH,
        };
        let json = serde_json::to_string(&body).unwrap();
        assert!(json.contains(r#""product":"storage-sifter""#));
        assert!(json.len() <= 2048);
    }

    /// Live: one real ping through `run`, with the id kept under
    /// `$XDG_CONFIG_HOME`. Run with `cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn live_ping_records_today() {
        run();
        let state: State =
            serde_json::from_str(&std::fs::read_to_string(state_path().unwrap()).unwrap()).unwrap();
        assert_eq!(state.last_ping, Some(utc_date(SystemTime::now())));
    }
}
