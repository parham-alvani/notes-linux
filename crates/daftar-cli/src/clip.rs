//! `clip`: a web page as a new note, by `daftar_core::web_fetch`.

use std::path::Path;

/// Fetches [url] and writes it as a new note in [folder]; the note's vault path.
pub fn run(root: &Path, url: &str, folder: &str, title: Option<&str>) -> Result<String, String> {
    let clip = daftar_core::web_fetch::clip(root, url, folder, title, concat!("daftar-cli/", env!("CARGO_PKG_VERSION")))?;
    for (address, why) in clip.skipped {
        eprintln!("daftar-cli: kept as a link, not saved: {address}: {why}");
    }
    Ok(clip.path)
}
