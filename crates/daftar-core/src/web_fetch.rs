//! A web page fetched and clipped into a new note: the page and its
//! pictures come over HTTP with `ureq` (rustls, so no system TLS library on
//! any OS), and `WebClip` makes the note. Behind the `web` feature, for the
//! command line's `clip` and the app's Clip Web Page.

use std::path::Path;
use std::time::Duration;

use ureq::ResponseExt;

use crate::{Day, WebClip};

/// The most a page, or a picture, may weigh.
const PAGE_LIMIT: u64 = 20 * 1024 * 1024;
const PICTURE_LIMIT: u64 = 30 * 1024 * 1024;

/// What a clip wrote: the note's vault path, and the pictures kept as links
/// with why.
pub struct Clip {
    pub path: String,
    pub skipped: Vec<(String, String)>,
}

/// Fetches [url] and writes it as a new note in [folder], named after the
/// page or [title]. [agent] names the program to the server.
pub fn clip(root: &Path, url: &str, folder: &str, title: Option<&str>, agent: &str) -> Result<Clip, String> {
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err(format!("{url} is not a web address (http:// or https://)"));
    }
    let agent: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(Duration::from_mins(1))).user_agent(agent).build().into();
    let (bytes, media, at) = fetch(&agent, url, PAGE_LIMIT)?;
    if let Some(media) = &media {
        let kind = media.split(';').next().unwrap_or("").trim().to_lowercase();
        if !kind.is_empty() && kind != "text/html" && kind != "application/xhtml+xml" {
            return Err(format!("{url} is not a web page ({kind})"));
        }
    }
    let html = String::from_utf8_lossy(&bytes);
    // Addresses on the page are read against where the redirects ended; the
    // note keeps the address given.
    let mut clipping = WebClip::extract(&html, &at);
    if let Some(title) = title.map(str::trim).filter(|t| !t.is_empty()) {
        clipping.title = title.to_string();
    }
    let picture = |address: &str| fetch(&agent, address, PICTURE_LIMIT).map(|(bytes, media, _)| (bytes, media));
    let saved = WebClip::save_clipping(root, url, clipping, folder, Day::today(), picture)?;
    Ok(Clip { path: saved.path, skipped: saved.skipped })
}

/// The body of [url], its media type, and the address it came from after redirects.
fn fetch(agent: &ureq::Agent, url: &str, limit: u64) -> Result<(Vec<u8>, Option<String>, String), String> {
    let mut response = agent.get(url).call().map_err(|e| format!("{url}: {e}"))?;
    let media = response.headers().get("content-type").and_then(|v| v.to_str().ok()).map(String::from);
    let at = response.get_uri().to_string();
    let bytes = response.body_mut().with_config().limit(limit).read_to_vec().map_err(|e| format!("{url}: {e}"))?;
    Ok((bytes, media, at))
}

/// What a web address answers: a status, or why nothing answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Answer {
    Status(u16),
    Unreachable(String),
}

fn probe_agent(agent: &str) -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(15)))
        .timeout_connect(Some(Duration::from_secs(8)))
        .http_status_as_error(false)
        .user_agent(agent)
        .build()
        .into()
}

/// [ask] on each of [items], [threads] at a time: the time goes in waiting.
fn each<T: Sync, R: Send>(items: &[T], threads: usize, ask: impl Fn(&T) -> R + Sync + Send) -> Vec<R> {
    use rayon::prelude::*;
    match rayon::ThreadPoolBuilder::new().num_threads(threads).build() {
        Ok(pool) => pool.install(|| items.par_iter().map(&ask).collect()),
        Err(_) => items.iter().map(ask).collect(),
    }
}

/// What each of [urls] answers: a HEAD, and a GET when the HEAD's answer is
/// not a yes, as many servers will not say to a HEAD what they say to a
/// GET. The bodies are not read.
pub fn answers(urls: &[String], agent: &str) -> Vec<Answer> {
    let agent = probe_agent(agent);
    each(urls, 32, |url| match agent.head(url).call() {
        Ok(response) if response.status().as_u16() < 400 => Answer::Status(response.status().as_u16()),
        Ok(_) => match agent.get(url).call() {
            Ok(response) => Answer::Status(response.status().as_u16()),
            Err(error) => Answer::Unreachable(error.to_string()),
        },
        Err(error) => Answer::Unreachable(error.to_string()),
    })
}

/// What GitHub's API says of each `owner/repo` in [repos] to [token]: 404
/// when it is not there or [token] cannot see it, 200 for a public one, 403
/// for a private one [token] can see.
pub fn github_repos(repos: &[String], token: &str, agent: &str) -> Vec<Answer> {
    let agent = probe_agent(agent);
    each(repos, 32, |repo| {
        let call = agent
            .get(&format!("https://api.github.com/repos/{repo}"))
            .header("Authorization", &format!("Bearer {token}"))
            .header("Accept", "application/vnd.github+json")
            .call();
        match call {
            Ok(mut response) if response.status().as_u16() == 200 => {
                let body = response.body_mut().with_config().limit(1024 * 1024).read_to_string().unwrap_or_default();
                let private = serde_json::from_str::<serde_json::Value>(&body).ok().and_then(|v| v["private"].as_bool()).unwrap_or(false);
                Answer::Status(if private { 403 } else { 200 })
            }
            Ok(response) => Answer::Status(response.status().as_u16()),
            Err(error) => Answer::Unreachable(error.to_string()),
        }
    })
}

/// The Wayback Machine's copy of each of [urls] closest to now, over https,
/// or `None` when it has none. Two at a time, and a 429 waits as long as
/// it asks (`Retry-After`, else a few seconds, longer each time) and asks
/// again: archive.org turns a crowd away.
pub fn archived(urls: &[String], agent: &str) -> Vec<Option<String>> {
    let agent = probe_agent(agent);
    each(urls, 2, |url| {
        for attempt in 1..=5 {
            let mut response = agent.get("https://archive.org/wayback/available").query("url", url).call().ok()?;
            match response.status().as_u16() {
                200 => {
                    let body = response.body_mut().with_config().limit(1024 * 1024).read_to_string().ok()?;
                    return snapshot(&body);
                }
                429 | 503 => {
                    let asked = response.headers().get("retry-after").and_then(|v| v.to_str().ok()?.trim().parse::<u64>().ok());
                    std::thread::sleep(Duration::from_secs(asked.unwrap_or(5 * attempt).min(60)));
                }
                _ => return None,
            }
        }
        None
    })
}

/// The snapshot in an answer of archive.org's availability API, over https.
fn snapshot(body: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(body).ok()?;
    let closest = &value["archived_snapshots"]["closest"];
    if closest["available"].as_bool() != Some(true) || closest["status"].as_str().is_some_and(|s| s != "200") {
        return None;
    }
    let url = closest["url"].as_str()?;
    Some(url.strip_prefix("http://").map_or_else(|| url.to_string(), |rest| format!("https://{rest}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_snapshot_is_read_from_the_wayback_machines_answer() {
        let found = r#"{"url":"x.io/p","archived_snapshots":{"closest":{"status":"200","available":true,"url":"http://web.archive.org/web/20200101000000/https://x.io/p","timestamp":"20200101000000"}}}"#;
        assert_eq!(snapshot(found).as_deref(), Some("https://web.archive.org/web/20200101000000/https://x.io/p"));
        assert_eq!(snapshot(r#"{"url":"x.io/p","archived_snapshots":{}}"#), None);
        let missing = r#"{"archived_snapshots":{"closest":{"status":"404","available":true,"url":"http://web.archive.org/web/1/x"}}}"#;
        assert_eq!(snapshot(missing), None);
    }
}
