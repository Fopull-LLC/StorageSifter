//! Links the app opens on fopull.com. Each carries a campaign tag naming the
//! spot in the app it came from, so Fopull can see which ones people use.
//! Nothing is sent by the app itself: the tag rides on the page the browser
//! opens.

const SOURCE: &str = "storagesifter-app";

/// The project page (downloads, guides).
pub fn project(medium: &str) -> String {
    format!("https://fopull.com/storage-sifter?utm_source={SOURCE}&utm_medium={medium}")
}

/// Fopull's home page.
pub fn home(medium: &str) -> String {
    format!("https://fopull.com/?utm_source={SOURCE}&utm_medium={medium}")
}

/// The pay-what-you-want support page, opened on StorageSifter.
pub fn support(medium: &str) -> String {
    format!(
        "https://fopull.com/support?product=storage-sifter&utm_source={SOURCE}&utm_medium={medium}"
    )
}

pub const ISSUES: &str = "https://github.com/Fopull-LLC/StorageSifter/issues";

/// The "you just updated" note, with its support line, shows at most once in
/// this many days.
pub const SUPPORT_LINE_EVERY_DAYS: u64 = 30;

/// Days since the Unix epoch, from the local clock.
pub fn today() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() / 86_400)
        .unwrap_or(0)
}

/// Should the note show on this launch? Only on the first launch of a new
/// version, never on a fresh install, and not again within the window.
pub fn support_line_due(
    last_version: &str,
    current: &str,
    last_shown_day: u64,
    today: u64,
) -> bool {
    !last_version.is_empty()
        && last_version != current
        && (last_shown_day == 0 || today >= last_shown_day + SUPPORT_LINE_EVERY_DAYS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_carry_their_campaign_tag() {
        assert!(support("help").contains("product=storage-sifter"));
        assert!(support("help").ends_with("utm_source=storagesifter-app&utm_medium=help"));
        assert!(project("update").starts_with("https://fopull.com/storage-sifter?"));
    }

    #[test]
    fn the_note_shows_once_per_update_and_at_most_monthly() {
        // A fresh install says nothing.
        assert!(!support_line_due("", "0.4.1", 0, 20_000));
        // Same version again: nothing.
        assert!(!support_line_due("0.4.1", "0.4.1", 0, 20_000));
        // First launch of a new version.
        assert!(support_line_due("0.4.0", "0.4.1", 0, 20_000));
        // Another update ten days after the last note: quiet.
        assert!(!support_line_due("0.4.1", "0.4.2", 20_000, 20_010));
        // Thirty days on: shows again.
        assert!(support_line_due("0.4.1", "0.4.2", 20_000, 20_030));
    }
}
