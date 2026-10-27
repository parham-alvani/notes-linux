//! `clip`: a web page as a new note. The page and its pictures come over
//! HTTP with `ureq` (rustls, so no system TLS library on any OS); the rest,
//! what the page's text is and how the note is written, is
//! `daftar_core::WebClip`'s.

use std::path::Path;
use std::time::Duration;

use daftar_core::{Day, WebClip};
use ureq::ResponseExt;

/// The most a page, or a picture, may weigh.
const PAGE_LIMIT: u64 = 20 * 1024 * 1024;
const PICTURE_LIMIT: u64 = 30 * 1024 * 1024;

/// Fetches [url] and writes it as a new note in [folder]; the note's vault path.
pub fn run(root: &Path, url: &str, folder: &str, title: Option<&str>) -> Result<String, String> {
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err(format!("{url} is not a web address (http:// or https://)"));
    }
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(60)))
        .user_agent(concat!("daftar-cli/", env!("CARGO_PKG_VERSION")))
        .build()
        .into();
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
    for (address, why) in saved.skipped {
        eprintln!("daftar-cli: kept as a link, not saved: {address}: {why}");
    }
    Ok(saved.path)
}

/// The body of [url], its media type, and the address it came from after redirects.
fn fetch(agent: &ureq::Agent, url: &str, limit: u64) -> Result<(Vec<u8>, Option<String>, String), String> {
    let mut response = agent.get(url).call().map_err(|e| format!("{url}: {e}"))?;
    let media = response.headers().get("content-type").and_then(|v| v.to_str().ok()).map(String::from);
    let at = response.get_uri().to_string();
    let bytes = response.body_mut().with_config().limit(limit).read_to_vec().map_err(|e| format!("{url}: {e}"))?;
    Ok((bytes, media, at))
}
