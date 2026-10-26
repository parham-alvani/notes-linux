//! The picture cleanup of `notes-cli`, ported: pictures in `uploads/` that
//! no note names are removed; the others become JPEGs under 1 MB, named
//! after the note that shows them and their content's hash
//! (`Note_1a2b3c4d.jpg`), and the notes are rewritten to the new names.
//!
//! A picture is kept when any note mentions its file name anywhere, even in
//! a code block: a word too many keeps a picture, never removes one.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use image::codecs::jpeg::JpegEncoder;
use image::{DynamicImage, ImageReader, Rgb, RgbImage};
use md5::{Digest, Md5};
use regex::Regex;
use walkdir::WalkDir;

/// 1 MB, the most a picture may weigh.
pub const TARGET_SIZE: u64 = 1024 * 1024;

const FORMATS: [&str; 7] = ["png", "jpg", "jpeg", "bmp", "webp", "tiff", "tif"];

pub struct Options {
    pub dry_run: bool,
    pub keep_originals: bool,
}

/// One picture and the notes that name it.
struct Picture {
    path: PathBuf,
    notes: Vec<PathBuf>,
}

/// A picture's old file and its new one; the same file when it was right.
struct Moved {
    from: PathBuf,
    to: PathBuf,
    notes: Vec<PathBuf>,
}

pub fn run(root: &Path, uploads: &Path, options: &Options) -> io::Result<()> {
    if options.dry_run {
        println!("Dry run: nothing is changed.\n");
    }
    let pictures = pictures_in(uploads)?;
    let notes = notes_in(root);
    println!("{} pictures in {}, {} notes", pictures.len(), uploads.display(), notes.len());
    let texts: Vec<(PathBuf, String)> =
        notes.into_iter().filter_map(|path| fs::read_to_string(&path).ok().map(|text| (path, text))).collect();

    let (mut used, mut unused) = (Vec::new(), Vec::new());
    for path in pictures {
        let name = file_name(&path);
        let notes: Vec<PathBuf> = texts.iter().filter(|(_, text)| text.contains(&name)).map(|(note, _)| note.clone()).collect();
        if notes.is_empty() { unused.push(path) } else { used.push(Picture { path, notes }) }
    }

    println!("\n1. Pictures no note names");
    let mut freed = 0;
    for path in &unused {
        let size = fs::metadata(path)?.len();
        freed += size;
        if options.dry_run {
            println!("  would remove {} ({})", file_name(path), kb(size));
        } else {
            fs::remove_file(path)?;
            println!("  removed {} ({})", file_name(path), kb(size));
        }
    }
    println!("  {} pictures, {}", unused.len(), mb(freed));

    println!("\n2. Pictures notes show");
    let mut moved = Vec::new();
    for picture in used {
        match optimize(&picture, options.dry_run) {
            Ok(Some(to)) => moved.push(Moved { from: picture.path, to, notes: picture.notes }),
            Ok(None) => {}
            Err(error) => println!("  {}: {error}", file_name(&picture.path)),
        }
    }

    println!("\n3. Notes");
    let (mut files, mut references) = (0, 0);
    for item in moved.iter().filter(|m| m.from != m.to) {
        let (old, new) = (file_name(&item.from), file_name(&item.to));
        for note in &item.notes {
            let Ok(text) = fs::read_to_string(note) else { continue };
            let Some(changed) = rewrite(&text, &old, &new) else { continue };
            references += 1;
            if options.dry_run {
                println!("  would update {} ({old} → {new})", file_name(note));
            } else {
                write_in_place(note, changed.as_bytes())?;
                files += 1;
                println!("  updated {} ({old} → {new})", file_name(note));
            }
        }
    }
    println!("  {references} references in {files} notes");

    println!("\n4. Originals");
    if options.keep_originals {
        println!("  kept (--keep-originals)");
    } else {
        let mut freed = 0;
        for item in moved.iter().filter(|m| m.from != m.to && m.from.exists() && (options.dry_run || m.to.exists())) {
            freed += fs::metadata(&item.from)?.len();
            if options.dry_run {
                println!("  would remove {}", file_name(&item.from));
            } else {
                fs::remove_file(&item.from)?;
                println!("  removed {}", file_name(&item.from));
            }
        }
        println!("  {}", mb(freed));
    }
    Ok(())
}

/// The new file of [picture], written unless [dry_run], or `None` when it
/// could not be made.
fn optimize(picture: &Picture, dry_run: bool) -> io::Result<Option<PathBuf>> {
    let path = &picture.path;
    if is_optimized(path)? {
        println!("  {} is done", file_name(path));
        return Ok(Some(path.clone()));
    }
    let size = fs::metadata(path)?.len();
    let base = picture
        .notes
        .first()
        .and_then(|n| n.file_stem())
        .map(|s| s.to_string_lossy().replace(' ', ""))
        .unwrap_or_else(|| "optimized".into());
    let jpeg = is_jpeg(path);
    if jpeg && size <= TARGET_SIZE {
        // Right already, but for its name.
        let to = path.with_file_name(format!("{base}_{}.jpg", hash(&fs::read(path)?)));
        if dry_run {
            println!("  would rename {} → {} ({})", file_name(path), file_name(&to), kb(size));
        } else {
            fs::rename(path, &to)?;
            println!("  renamed {} → {} ({})", file_name(path), file_name(&to), kb(size));
        }
        return Ok(Some(to));
    }
    let image = match ImageReader::open(path)?.with_guessed_format()?.decode() {
        Ok(image) => image,
        Err(error) => {
            println!("  {} could not be read: {error}", file_name(path));
            return Ok(None);
        }
    };
    let (bytes, quality) = compress(&image);
    // Named by the hash of what is written, so the next run finds it done.
    let to = path.with_file_name(format!("{base}_{}.jpg", hash(&bytes)));
    let verb = if dry_run { "would make" } else { "made" };
    if !dry_run {
        fs::write(&to, &bytes)?;
    }
    let warning = if bytes.len() as u64 > TARGET_SIZE { ", still over 1 MB" } else { "" };
    println!("  {verb} {} → {} (quality {quality}, {} → {}{warning})", file_name(path), file_name(&to), kb(size), kb(bytes.len() as u64));
    Ok(Some(to))
}

/// [image] as a JPEG on white, at the best quality under 1 MB, and that quality.
pub fn compress(image: &DynamicImage) -> (Vec<u8>, u8) {
    let rgb = on_white(image);
    let mut quality: u8 = 95;
    loop {
        let mut bytes = Vec::new();
        let encoded = JpegEncoder::new_with_quality(&mut bytes, quality).encode_image(&rgb);
        let size = bytes.len() as u64;
        if encoded.is_err() || size <= TARGET_SIZE || quality <= 10 {
            return (bytes, quality);
        }
        // Lower faster while far over.
        let step = if size > TARGET_SIZE * 2 {
            10
        } else if size * 2 > TARGET_SIZE * 3 {
            5
        } else {
            3
        };
        quality = quality.saturating_sub(step).max(10);
    }
}

/// The picture with what is see-through laid on white, as a JPEG has no alpha.
fn on_white(image: &DynamicImage) -> RgbImage {
    if !image.color().has_alpha() {
        return image.to_rgb8();
    }
    let rgba = image.to_rgba8();
    RgbImage::from_fn(rgba.width(), rgba.height(), |x, y| {
        let pixel = rgba.get_pixel(x, y).0;
        let alpha = pixel[3] as u32;
        let mix = |c: u8| ((c as u32 * alpha + 255 * (255 - alpha)) / 255) as u8;
        Rgb([mix(pixel[0]), mix(pixel[1]), mix(pixel[2])])
    })
}

/// A JPEG under 1 MB named `{name}_{hash}.jpg` by its own content's hash.
fn is_optimized(path: &Path) -> io::Result<bool> {
    if !is_jpeg(path) || fs::metadata(path)?.len() > TARGET_SIZE {
        return Ok(false);
    }
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let Some((_, named)) = stem.rsplit_once('_') else { return Ok(false) };
    if named.len() != 8 || !named.chars().all(|c| c.is_ascii_hexdigit()) {
        return Ok(false);
    }
    Ok(hash(&fs::read(path)?) == named.to_lowercase())
}

/// The first 8 hex digits of the MD5 of [bytes], as `notes-cli` names files.
pub fn hash(bytes: &[u8]) -> String {
    let digest = Md5::digest(bytes);
    digest.iter().take(4).map(|b| format!("{b:02x}")).collect()
}

/// [text] with [old] renamed to [new], in every way a note names a picture,
/// or `None` when it does not name it.
pub fn rewrite(text: &str, old: &str, new: &str) -> Option<String> {
    if !text.contains(old) {
        return None;
    }
    let mut out = text.to_string();
    for (from, to) in [
        (format!("uploads/{old}"), format!("uploads/{new}")),
        (format!("./uploads/{old}"), format!("uploads/{new}")),
        (format!("/uploads/{old}"), format!("/uploads/{new}")),
        (format!("]({old})"), format!("](uploads/{new})")),
    ] {
        out = out.replace(&from, &to);
    }
    // `[[old|name]]` and `![[old]]`: the folder added, the name kept.
    let wiki = Regex::new(&format!(r"(!?\[\[){}(\|[^\]]+)?\]\]", regex::escape(old))).ok()?;
    out = wiki
        .replace_all(&out, |c: &regex::Captures| format!("{}uploads/{new}{}]]", &c[1], c.get(2).map_or("", |m| m.as_str())))
        .into_owned();
    (out != text).then_some(out)
}

fn pictures_in(folder: &Path) -> io::Result<Vec<PathBuf>> {
    let mut out: Vec<PathBuf> = fs::read_dir(folder)?
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|path| path.is_file() && path.extension().is_some_and(|e| FORMATS.contains(&e.to_string_lossy().to_lowercase().as_str())))
        .collect();
    out.sort();
    Ok(out)
}

/// Every note in the vault, but in `.git`, `.obsidian` and the like.
fn notes_in(root: &Path) -> Vec<PathBuf> {
    WalkDir::new(root)
        .into_iter()
        .filter_entry(|e| e.depth() == 0 || !e.file_name().to_string_lossy().starts_with('.') && e.file_name() != "node_modules")
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file() && e.path().extension().is_some_and(|x| x == "md"))
        .map(|e| e.into_path())
        .collect()
}

/// Through a temporary file and a rename, so a crash never leaves half a note.
fn write_in_place(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let temporary = path.with_file_name(format!(".{}.daftar-{}", file_name(path), std::process::id()));
    fs::write(&temporary, bytes)?;
    fs::rename(&temporary, path).inspect_err(|_| {
        let _ = fs::remove_file(&temporary);
    })
}

fn is_jpeg(path: &Path) -> bool {
    path.extension().is_some_and(|e| matches!(e.to_string_lossy().to_lowercase().as_str(), "jpg" | "jpeg"))
}

fn file_name(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

fn kb(bytes: u64) -> String {
    format!("{:.1} KB", bytes as f64 / 1024.0)
}

fn mb(bytes: u64) -> String {
    format!("{:.1} MB", bytes as f64 / 1024.0 / 1024.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    // The tests of notes-cli's markdown_updater, by the same names.

    fn renamed(text: &str) -> String {
        rewrite(text, "original.png", "optimized.png").unwrap_or_else(|| text.to_string())
    }

    #[test]
    fn simple_uploads_prefix() {
        assert_eq!(renamed("![image](uploads/original.png)"), "![image](uploads/optimized.png)");
    }

    #[test]
    fn simple_relative_uploads_prefix() {
        assert_eq!(renamed("![image](./uploads/original.png)"), "![image](./uploads/optimized.png)");
    }

    #[test]
    fn simple_absolute_uploads_prefix() {
        assert_eq!(renamed("![image](/uploads/original.png)"), "![image](/uploads/optimized.png)");
    }

    #[test]
    fn wiki_style_links_with_and_without_display_name() {
        assert_eq!(renamed("[[original.png]]"), "[[uploads/optimized.png]]");
        assert_eq!(renamed("![[original.png]]"), "![[uploads/optimized.png]]");
        assert_eq!(renamed("[[original.png|My Image]]"), "[[uploads/optimized.png|My Image]]");
        assert_eq!(renamed("![[original.png|My Image]]"), "![[uploads/optimized.png|My Image]]");
    }

    #[test]
    fn standard_markdown_without_uploads_prefix() {
        assert_eq!(renamed("![alt text](original.png)"), "![alt text](uploads/optimized.png)");
    }

    #[test]
    fn multiple_references_in_same_file() {
        let out = renamed("![img1](original.png)\n[[original.png]]\n![[original.png]]\n");
        assert_eq!(out, "![img1](uploads/optimized.png)\n[[uploads/optimized.png]]\n![[uploads/optimized.png]]\n");
    }

    #[test]
    fn no_update_when_the_note_does_not_name_it() {
        assert_eq!(rewrite("# Header\nSome text", "original.png", "optimized.png"), None);
    }

    #[test]
    fn preserves_unrelated_content() {
        let out = renamed("# Header\n\nSome text\n\n![image](uploads/original.png)\n\nMore text\n- List item 1\n- List item 2\n");
        assert_eq!(out, "# Header\n\nSome text\n\n![image](uploads/optimized.png)\n\nMore text\n- List item 1\n- List item 2\n");
    }

    #[test]
    fn special_characters_in_filename() {
        assert_eq!(
            rewrite("![[my-image_2024(1).png]]", "my-image_2024(1).png", "my-image_2024(1)-opt.png").unwrap(),
            "![[uploads/my-image_2024(1)-opt.png]]"
        );
    }

    #[test]
    fn the_hash_is_md5s_first_eight_digits() {
        assert_eq!(hash(b""), "d41d8cd9");
    }

    #[test]
    fn a_transparent_picture_is_laid_on_white() {
        let clear = DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(2, 2, image::Rgba([0, 0, 0, 0])));
        assert_eq!(on_white(&clear).get_pixel(0, 0).0, [255, 255, 255]);
        let (bytes, quality) = compress(&clear);
        assert_eq!(quality, 95);
        assert!(bytes.starts_with(&[0xFF, 0xD8]));
    }

    #[test]
    fn a_run_cleans_up_and_a_second_run_changes_nothing() {
        let root = tempfile::tempdir().unwrap();
        let uploads = root.path().join("uploads");
        fs::create_dir_all(&uploads).unwrap();
        let picture = RgbImage::from_pixel(4, 4, Rgb([10, 200, 30]));
        picture.save(uploads.join("shown.png")).unwrap();
        picture.save(uploads.join("unused.png")).unwrap();
        fs::write(root.path().join("My Note.md"), "# Note\n![[shown.png]]\n").unwrap();
        let options = Options { dry_run: false, keep_originals: false };
        run(root.path(), &uploads, &options).unwrap();
        let left: Vec<String> = pictures_in(&uploads).unwrap().iter().map(|p| file_name(p)).collect();
        assert_eq!(left.len(), 1);
        assert!(left[0].starts_with("MyNote_") && left[0].ends_with(".jpg"), "{left:?}");
        assert_eq!(fs::read_to_string(root.path().join("My Note.md")).unwrap(), format!("# Note\n![[uploads/{}]]\n", left[0]));
        run(root.path(), &uploads, &options).unwrap();
        let again: Vec<String> = pictures_in(&uploads).unwrap().iter().map(|p| file_name(p)).collect();
        assert_eq!(again, left);
    }
}
