//! `daftar-cli export`: a note as a PDF, set by Typst inside this binary
//! (`TypstNote` is the note as Typst, `export.typ` the page). Its fonts are
//! built in, Vazirmatn for the text and DejaVu Sans Mono for code and the
//! symbols Vazirmatn lacks (`fonts/`, each with its license), so the file is
//! the same on every computer. The app's Export as PDF runs this.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use daftar_core::typst_note::right_to_left;
use daftar_core::{ConfigFile, Git, TypstNote, Vault, VaultIndex};
use typst::diag::{FileError, FileResult};
use typst::foundations::{Bytes, Datetime, Duration};
use typst::syntax::{FileId, RootedPath, Source, VirtualPath, VirtualRoot};
use typst::text::{Font, FontBook};
use typst::utils::LazyHash;
use typst::{Library, LibraryExt, World, WorldExt};
use typst_layout::PagedDocument;

const TEMPLATE: &str = include_str!("export.typ");
const FONTS: [&[u8]; 4] = [
    include_bytes!("../fonts/Vazirmatn-Regular.ttf"),
    include_bytes!("../fonts/Vazirmatn-Bold.ttf"),
    include_bytes!("../fonts/DejaVuSansMono.ttf"),
    include_bytes!("../fonts/DejaVuSansMono-Bold.ttf"),
];

/// What the head and the foot of the page show.
pub struct Look {
    /// The person's logo, `export.logo`.
    pub logo: Option<PathBuf>,
    /// The logo of the folder the note is in, when it has one.
    pub folder_logo: bool,
    /// `export.footer`.
    pub footer: String,
}

impl Look {
    pub fn from_config(config: &ConfigFile) -> Look {
        Look { logo: config.path("export.logo"), folder_logo: true, footer: config.string("export.footer").unwrap_or_default() }
    }
}

/// The note at [path] as a PDF at [output].
pub fn run(root: &Path, index: &VaultIndex, path: &str, output: &Path, look: &Look) -> Result<(), String> {
    let text = fs::read_to_string(root.join(path)).map_err(|e| format!("{path}: {e}"))?;
    let note = TypstNote::new(&text, path, |target, source| index.resolve(target, source));
    let title = Vault::title(path);
    // The head has the title: a first heading that says it again goes.
    let repeated = format!("#heading(level: 1)[#{}]\n\n", TypstNote::string(&title));
    let body = note.markup.strip_prefix(&repeated).unwrap_or(&note.markup);
    let mut files: HashMap<String, PathBuf> = HashMap::new();
    for (number, picture) in note.pictures.iter().enumerate() {
        let extension = picture.rsplit_once('.').map_or("", |(_, e)| e).to_lowercase();
        files.insert(format!("/pictures/{number}.{extension}"), root.join(picture));
    }
    let mut logo = "none".to_string();
    if let Some(file) = &look.logo {
        if !file.is_file() {
            return Err(format!("export.logo: {} is not a file", file.display()));
        }
        let name = logo_name("logo", file);
        logo = TypstNote::string(&name);
        files.insert(name, file.clone());
    }
    let mut folder_logo = "none".to_string();
    let mut folder = None;
    if look.folder_logo
        && let Some((at, file)) = TypstNote::folder_logo(path, &index.files)
    {
        let name = logo_name("folder-logo", Path::new(&file));
        folder_logo = TypstNote::string(&name);
        files.insert(name, root.join(&file));
        folder = Some(Vault::title(&at));
    }
    let subtitle = [folder, changed(root, path)].into_iter().flatten().collect::<Vec<_>>().join(" · ");
    let main = format!(
        "#import \"/export.typ\": *\n#show: note.with(title: {}, subtitle: {}, logo: {logo}, folder-logo: {folder_logo}, footer: {}, title-rtl: {})\n\n{}",
        TypstNote::string(&title),
        TypstNote::string(&subtitle),
        TypstNote::string(&look.footer),
        right_to_left(&title),
        body,
    );
    let world = Export::new(main, files);
    let warned = typst::compile::<PagedDocument>(&world);
    let document = warned.output.map_err(|errors| problems(&world, &errors))?;
    let pdf = typst_pdf::pdf(&document, &typst_pdf::PdfOptions::default()).map_err(|errors| problems(&world, &errors))?;
    write(output, &pdf)
}

/// The day the note last changed: its last commit, else its file's time.
fn changed(root: &Path, path: &str) -> Option<String> {
    if let Ok(history) = Git::new(root.to_path_buf()).history(path, 1)
        && let Some(last) = history.first()
    {
        return Some(last.date.format("%Y-%m-%d").to_string());
    }
    let modified = fs::metadata(root.join(path)).and_then(|m| m.modified()).ok()?;
    Some(chrono::DateTime::<chrono::Local>::from(modified).format("%Y-%m-%d").to_string())
}

fn logo_name(name: &str, file: &Path) -> String {
    let extension = file.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    format!("/logos/{name}.{extension}")
}

/// Through a temporary file and a rename, as every write of Daftar's.
fn write(output: &Path, pdf: &[u8]) -> Result<(), String> {
    let name = output.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let temporary = output.with_file_name(format!(".{name}.daftar-{}", std::process::id()));
    fs::write(&temporary, pdf).map_err(|e| format!("{}: {e}", temporary.display()))?;
    fs::rename(&temporary, output).map_err(|e| {
        let _ = fs::remove_file(&temporary);
        format!("{}: {e}", output.display())
    })
}

/// Typst's errors, one a line, with the line of the page they are on.
fn problems(world: &Export, errors: &[typst::diag::SourceDiagnostic]) -> String {
    let lines: Vec<String> = errors
        .iter()
        .map(|error| {
            let at = world.range(error.span).and_then(|range| {
                let source = world.source(world.main).ok()?;
                let line = source.lines().byte_to_line(range.start)? + 1;
                Some(format!(" (line {line} of the page)"))
            });
            format!("{}{}", error.message, at.unwrap_or_default())
        })
        .collect();
    format!("the PDF was not made: {}", lines.join("; "))
}

/// What Typst reads: the page, the template, the pictures and the fonts.
struct Export {
    library: LazyHash<Library>,
    book: LazyHash<FontBook>,
    fonts: Vec<Font>,
    main: FileId,
    sources: HashMap<FileId, Source>,
    files: HashMap<String, PathBuf>,
}

impl Export {
    fn new(main: String, files: HashMap<String, PathBuf>) -> Export {
        let fonts: Vec<Font> = FONTS.into_iter().flat_map(|data| Font::iter(Bytes::new(data))).collect();
        let id = |path: &str| FileId::new(RootedPath::new(VirtualRoot::Project, VirtualPath::new(path).expect("a path of ours")));
        let (main_id, template_id) = (id("/main.typ"), id("/export.typ"));
        let sources = HashMap::from([(main_id, Source::new(main_id, main)), (template_id, Source::new(template_id, TEMPLATE.to_string()))]);
        Export {
            library: LazyHash::new(Library::default()),
            book: LazyHash::new(FontBook::from_fonts(&fonts)),
            fonts,
            main: main_id,
            sources,
            files,
        }
    }
}

impl World for Export {
    fn library(&self) -> &LazyHash<Library> {
        &self.library
    }

    fn book(&self) -> &LazyHash<FontBook> {
        &self.book
    }

    fn main(&self) -> FileId {
        self.main
    }

    fn source(&self, id: FileId) -> FileResult<Source> {
        self.sources.get(&id).cloned().ok_or(FileError::NotSource)
    }

    fn file(&self, id: FileId) -> FileResult<Bytes> {
        if let Some(source) = self.sources.get(&id) {
            return Ok(Bytes::from_string(source.clone()));
        }
        let name = id.vpath().get_with_slash();
        let file = self.files.get(name).ok_or_else(|| FileError::NotFound(PathBuf::from(name)))?;
        fs::read(file).map(Bytes::new).map_err(|_| FileError::NotFound(file.clone()))
    }

    fn font(&self, index: usize) -> Option<Font> {
        self.fonts.get(index).cloned()
    }

    fn today(&self, _offset: Option<Duration>) -> Option<Datetime> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOGO: &str =
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10"><rect width="10" height="10" fill="#a85200"/></svg>"##;

    #[test]
    fn a_note_with_both_logos_and_a_picture_is_a_pdf() {
        let root = tempfile::tempdir().unwrap();
        let root = root.path();
        fs::create_dir_all(root.join("Companies/Acme/Platform")).unwrap();
        fs::write(root.join("Companies/Acme/logo.svg"), LOGO).unwrap();
        fs::write(root.join("Companies/Acme/chart.svg"), LOGO).unwrap();
        fs::write(root.join("me.svg"), LOGO).unwrap();
        let note = "# Queue\n\nSee [[Acme]], ==this== and ![[chart.svg|40]].\n\n- [ ] Call 📅 2026-11-02\n\n## سرآغاز\n\nمتن فارسی.\n";
        fs::write(root.join("Companies/Acme/Platform/Queue.md"), note).unwrap();
        fs::write(root.join("Companies/Acme/Acme.md"), "# Acme\n").unwrap();
        let index = Vault::index(root, None);
        let output = root.join("out/Queue.pdf");
        fs::create_dir_all(output.parent().unwrap()).unwrap();
        let look = Look { logo: Some(root.join("me.svg")), folder_logo: true, footer: "A footer".to_string() };
        run(root, &index, "Companies/Acme/Platform/Queue.md", &output, &look).unwrap();
        let pdf = fs::read(&output).unwrap();
        assert!(pdf.starts_with(b"%PDF-"));
        // No date in the file: the same note makes the same PDF.
        run(root, &index, "Companies/Acme/Platform/Queue.md", &root.join("out/again.pdf"), &look).unwrap();
        assert_eq!(fs::read(root.join("out/again.pdf")).unwrap(), pdf);
    }

    #[test]
    fn a_logo_that_is_not_there_is_refused() {
        let root = tempfile::tempdir().unwrap();
        let root = root.path();
        fs::write(root.join("Note.md"), "Text.\n").unwrap();
        let index = Vault::index(root, None);
        let look = Look { logo: Some(root.join("missing.svg")), folder_logo: true, footer: String::new() };
        let error = run(root, &index, "Note.md", &root.join("Note.pdf"), &look).unwrap_err();
        assert!(error.starts_with("export.logo: "), "{error}");
        assert!(!root.join("Note.pdf").exists());
    }
}
