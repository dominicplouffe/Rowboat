//! Update check: asks GitHub which releases exist and says whether one is
//! newer. That request is the only thing sent; nothing about the user, their
//! connections or their usage goes with it, and the check can be turned off.
//!
//! Installing is left to the user (the notice opens the release page).
//! Unattended installs need signed update artifacts, which do not exist yet.

use std::cmp::Ordering;
use std::time::Duration;

use serde::Serialize;

use crate::settings::UpdateChannel;

pub const RELEASES_API: &str = "https://api.github.com/repos/dominicplouffe/Rowboat/releases";
/// The only place the "update available" notice may send the browser.
pub const RELEASE_PAGE_PREFIX: &str = "https://github.com/dominicplouffe/Rowboat/releases/";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    pub version: String,
    pub url: String,
    pub prerelease: bool,
}

/// `major.minor.patch` plus an optional pre-release tag, e.g. `0.2.0-beta.1`.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Version {
    core: [u64; 3],
    pre: Option<String>,
}

impl Version {
    fn parse(text: &str) -> Option<Self> {
        let text = text.trim().trim_start_matches('v');
        let (core, pre) = match text.split_once('-') {
            Some((core, pre)) => (core, Some(pre.to_string())),
            None => (text, None),
        };
        let mut parts = core.split('.').map(|part| part.parse::<u64>().ok());
        let version = [
            parts.next()??,
            parts.next().unwrap_or(Some(0))?,
            parts.next().unwrap_or(Some(0))?,
        ];
        if parts.next().is_some() {
            return None;
        }
        Some(Self { core: version, pre })
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        self.core
            .cmp(&other.core)
            .then_with(|| match (&self.pre, &other.pre) {
                (None, None) => Ordering::Equal,
                // 1.0.0 is newer than 1.0.0-beta.
                (None, Some(_)) => Ordering::Greater,
                (Some(_), None) => Ordering::Less,
                (Some(a), Some(b)) => compare_pre(a, b),
            })
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Dot-separated identifiers; numeric ones compare as numbers (beta.10 > beta.2).
fn compare_pre(a: &str, b: &str) -> Ordering {
    let mut left = a.split('.');
    let mut right = b.split('.');
    loop {
        match (left.next(), right.next()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) => {
                let order = match (x.parse::<u64>(), y.parse::<u64>()) {
                    (Ok(x), Ok(y)) => x.cmp(&y),
                    (Ok(_), Err(_)) => Ordering::Less,
                    (Err(_), Ok(_)) => Ordering::Greater,
                    (Err(_), Err(_)) => x.cmp(y),
                };
                if order != Ordering::Equal {
                    return order;
                }
            }
        }
    }
}

/// The newest published release that is newer than `current`, if any.
/// The stable channel ignores pre-releases; drafts are never offered.
pub fn newest(
    releases: &serde_json::Value,
    current: &str,
    channel: UpdateChannel,
) -> Option<UpdateInfo> {
    let current = Version::parse(current)?;
    releases
        .as_array()?
        .iter()
        .filter_map(|release| {
            if release["draft"].as_bool().unwrap_or(false) {
                return None;
            }
            let prerelease = release["prerelease"].as_bool().unwrap_or(false);
            let version = Version::parse(release["tag_name"].as_str()?)?;
            // The tag decides too: a release someone forgot to flag is still a pre-release.
            let prerelease = prerelease || version.pre.is_some();
            if prerelease && channel == UpdateChannel::Stable {
                return None;
            }
            let url = release["html_url"].as_str()?;
            // Never hand the UI a link that leads anywhere else.
            if !url.starts_with(RELEASE_PAGE_PREFIX) {
                return None;
            }
            let name = release["tag_name"]
                .as_str()?
                .trim_start_matches('v')
                .to_string();
            (version > current).then(|| (version, name, url.to_string(), prerelease))
        })
        .max_by(|a, b| a.0.cmp(&b.0))
        .map(|(_, version, url, prerelease)| UpdateInfo {
            version,
            url,
            prerelease,
        })
}

/// Fetches the release list from `api` and compares it with `current`.
pub async fn check(
    api: &str,
    current: &str,
    channel: UpdateChannel,
) -> Result<Option<UpdateInfo>, String> {
    crate::tls::ensure_provider();
    let http = reqwest::Client::builder()
        .user_agent("rowboat-update-check")
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| format!("Could not set up the HTTP client: {e}"))?;
    let response = http
        .get(api)
        .query(&[("per_page", "30")])
        .header("accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| format!("Could not reach GitHub: {}", e.without_url()))?;
    if !response.status().is_success() {
        return Err(format!(
            "GitHub answered with HTTP {}.",
            response.status().as_u16()
        ));
    }
    let releases: serde_json::Value = response
        .json()
        .await
        .map_err(|e| format!("GitHub's answer could not be read: {}", e.without_url()))?;
    Ok(newest(&releases, current, channel))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn release(tag: &str, prerelease: bool, draft: bool) -> serde_json::Value {
        json!({
            "tag_name": tag,
            "prerelease": prerelease,
            "draft": draft,
            "html_url": format!("{RELEASE_PAGE_PREFIX}tag/{tag}"),
        })
    }

    #[test]
    fn versions_order_like_semver() {
        let v = |s| Version::parse(s).unwrap();
        assert!(v("0.2.0") > v("0.1.9"));
        assert!(v("v1.0.0") > v("0.99.99"));
        assert!(v("1.0.0") > v("1.0.0-beta.1"));
        assert!(v("1.0.0-beta.10") > v("1.0.0-beta.2"));
        assert!(v("1.0.0-rc.1") > v("1.0.0-beta.9"));
        assert_eq!(v("0.1"), v("0.1.0"));
        assert!(Version::parse("nightly").is_none());
        assert!(Version::parse("1.2.3.4").is_none());
    }

    #[test]
    fn the_stable_channel_offers_the_newest_stable_release() {
        let releases = json!([
            release("v0.3.0-beta.1", true, false),
            release("v0.2.0", false, false),
            release("v0.1.1", false, false),
            release("v9.9.9", false, true),
        ]);
        let found = newest(&releases, "0.1.0", UpdateChannel::Stable).unwrap();
        assert_eq!(found.version, "0.2.0");
        assert!(!found.prerelease);
        assert!(found.url.ends_with("/tag/v0.2.0"));
    }

    #[test]
    fn the_beta_channel_also_offers_pre_releases() {
        let releases = json!([
            release("v0.3.0-beta.1", true, false),
            release("v0.2.0", false, false)
        ]);
        let found = newest(&releases, "0.2.0", UpdateChannel::Beta).unwrap();
        assert_eq!(found.version, "0.3.0-beta.1");
        assert!(found.prerelease);
        assert_eq!(newest(&releases, "0.2.0", UpdateChannel::Stable), None);
    }

    #[test]
    fn a_pre_release_tag_that_was_not_flagged_is_still_kept_off_stable() {
        let releases = json!([release("v0.2.0-rc.1", false, false)]);
        assert_eq!(newest(&releases, "0.1.0", UpdateChannel::Stable), None);
    }

    #[test]
    fn nothing_is_offered_when_up_to_date_or_when_the_answer_is_odd() {
        let releases = json!([
            release("v0.1.0", false, false),
            release("garbage", false, false)
        ]);
        assert_eq!(newest(&releases, "0.1.0", UpdateChannel::Stable), None);
        assert_eq!(
            newest(
                &json!({"message": "rate limited"}),
                "0.1.0",
                UpdateChannel::Stable
            ),
            None
        );
    }

    #[test]
    fn a_link_that_leaves_the_release_page_is_never_offered() {
        let mut odd = release("v0.2.0", false, false);
        odd["html_url"] = json!("https://example.com/evil");
        assert_eq!(newest(&json!([odd]), "0.1.0", UpdateChannel::Stable), None);
    }

    #[tokio::test]
    async fn the_check_reads_a_release_list_over_http() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let api = format!("http://{}/releases", listener.local_addr().unwrap());
        let body = json!([release("v0.2.0", false, false)]).to_string();
        let seen = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buf = vec![0u8; 4096];
            let n = socket.read(&mut buf).await.unwrap();
            let head = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            socket.write_all(head.as_bytes()).await.unwrap();
            String::from_utf8_lossy(&buf[..n]).to_string()
        });
        let found = check(&api, "0.1.0", UpdateChannel::Stable)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(found.version, "0.2.0");
        let request = seen.await.unwrap().to_lowercase();
        // Nothing identifying goes out: no version, no client id, no cookies.
        assert!(request.contains("user-agent: rowboat-update-check\r\n"));
        assert!(!request.contains("cookie") && !request.contains("0.1.0"));
    }
}
