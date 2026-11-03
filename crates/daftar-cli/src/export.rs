//! `daftar-cli export`: a note as a PDF, set by Typst inside this binary
//! (`TypstNote` is the note as Typst, `export.typ` the page). Its fonts are
//! built in, Vazirmatn for the text, DejaVu Sans Mono for code and the
//! symbols Vazirmatn lacks and New Computer Modern Math for math (`fonts/`,
//! each with its license), so the file is the same on every computer. Math
//! is TeX turned into Typst by MiTeX, with its Typst scope (`mitex/`, its
//! `specs`); a Mermaid block is drawn to SVG by mermaid-rs-renderer. Math
//! Typst cannot read and a diagram that does not parse stay their text. The
//! app's Export as PDF runs this.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use chrono::Datelike;
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
pub(crate) const FONTS: [&[u8]; 5] = [
    include_bytes!("../fonts/Vazirmatn-Regular.ttf"),
    include_bytes!("../fonts/Vazirmatn-Bold.ttf"),
    include_bytes!("../fonts/DejaVuSansMono.ttf"),
    include_bytes!("../fonts/DejaVuSansMono-Bold.ttf"),
    include_bytes!("../fonts/NewCMMath-Regular.otf"),
];
/// MiTeX's Typst scope, which the math it writes calls.
const MITEX: [(&str, &str); 3] = [
    ("/mitex/specs/mod.typ", include_str!("../mitex/specs/mod.typ")),
    ("/mitex/specs/prelude.typ", include_str!("../mitex/specs/prelude.typ")),
    ("/mitex/specs/latex/standard.typ", include_str!("../mitex/specs/latex/standard.typ")),
];
/// The family a diagram is measured in: no computer has it, so the renderer
/// sizes its labels by its own table, the same everywhere; the SVG then
/// names Vazirmatn, which Typst has.
const DIAGRAM_FONT: &str = "Daftar Diagram";

/// What the head and the foot of the page show.
pub struct Look {
    /// The person's logo, `export.logo`.
    pub logo: Option<PathBuf>,
    /// The logo of the folder the note is in, when it has one.
    pub folder_logo: bool,
    /// `export.footer`.
    pub footer: String,
    /// The day of the export, at the foot of every page and in the PDF's
    /// properties.
    pub day: chrono::NaiveDate,
}

impl Look {
    pub fn from_config(config: &ConfigFile) -> Look {
        Look {
            logo: config.path("export.logo"),
            folder_logo: true,
            footer: config.string("export.footer").unwrap_or_default(),
            day: export_day(std::env::var("SOURCE_DATE_EPOCH").ok().as_deref()),
        }
    }
}

/// The day of an export: [source_date_epoch]'s (`$SOURCE_DATE_EPOCH`, in
/// seconds), for a build that must make the same bytes on another day, else
/// today. A day and no time, so one note makes one PDF all day.
fn export_day(source_date_epoch: Option<&str>) -> chrono::NaiveDate {
    source_date_epoch
        .and_then(|seconds| seconds.trim().parse::<i64>().ok())
        .and_then(|seconds| chrono::DateTime::from_timestamp(seconds, 0))
        .map_or_else(|| chrono::Local::now().date_naive(), |time| time.date_naive())
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
    let subtitle = [folder, changed(root, path).map(|day| format!("Changed {day}"))].into_iter().flatten().collect::<Vec<_>>().join(" · ");
    let main = format!(
        "#import \"/export.typ\": *\n#show: note.with(title: {}, subtitle: {}, logo: {logo}, folder-logo: {folder_logo}, footer: {}, title-rtl: {}, exported: datetime(year: {}, month: {}, day: {}))\n\n{}",
        TypstNote::string(&title),
        TypstNote::string(&subtitle),
        TypstNote::string(&look.footer),
        right_to_left(&title),
        look.day.year(),
        look.day.month(),
        look.day.day(),
        body,
    );
    let mut formulas: Vec<Option<String>> = note.formulas.iter().map(|tex| mitex::convert_math(tex, None).ok()).collect();
    let mut drawings = HashMap::new();
    let diagrams: Vec<Option<String>> = note
        .diagrams
        .iter()
        .enumerate()
        .map(|(number, source)| {
            let name = format!("/diagrams/{number}.svg");
            drawings.insert(name.clone(), Bytes::from_string(diagram(source)?));
            Some(name)
        })
        .collect();
    let world = Export::new(main.clone(), files.clone(), extras(&note, &formulas, &diagrams), drawings.clone());
    let mut warned = typst::compile::<PagedDocument>(&world);
    if warned.output.is_err() && formulas.iter().any(Option::is_some) {
        // Math MiTeX wrote but Typst cannot read stays its TeX.
        for formula in &mut formulas {
            if formula.as_deref().is_some_and(|code| !readable(code)) {
                *formula = None;
            }
        }
        let again = Export::new(main, files, extras(&note, &formulas, &diagrams), drawings);
        warned = typst::compile::<PagedDocument>(&again);
    }
    let document = warned.output.map_err(|errors| problems(&world, &errors))?;
    let pdf = typst_pdf::pdf(&document, &typst_pdf::PdfOptions::default()).map_err(|errors| problems(&world, &errors))?;
    write(output, &pdf)
}

/// [source] as an SVG in the page's colors, or None when it does not parse.
fn diagram(source: &str) -> Option<String> {
    let mut options = mermaid_rs_renderer::RenderOptions::default();
    options.theme.font_family = DIAGRAM_FONT.to_string();
    let svg = mermaid_rs_renderer::render_with_options(source, options).ok()?;
    Some(svg.replace(DIAGRAM_FONT, "Vazirmatn"))
}

/// `/extras.typ`: each formula's Typst math (none when MiTeX could not
/// read it) and TeX, and each diagram's SVG (none when it did not parse)
/// and text, which `export.typ` imports.
fn extras(note: &TypstNote, formulas: &[Option<String>], diagrams: &[Option<String>]) -> String {
    let or_none = |value: &Option<String>| value.as_deref().map_or_else(|| "none".to_string(), TypstNote::string);
    let formulas: String = note
        .formulas
        .iter()
        .zip(formulas)
        .map(|(tex, code)| format!("(code: {}, tex: {}),", or_none(code), TypstNote::string(tex)))
        .collect();
    let diagrams: String = note
        .diagrams
        .iter()
        .zip(diagrams)
        .map(|(source, svg)| format!("(svg: {}, source: {}),", or_none(svg), TypstNote::string(source)))
        .collect();
    format!("#let formulas = ({formulas})\n#let diagrams = ({diagrams})\n")
}

/// Whether Typst reads [code], MiTeX's math, on its own.
fn readable(code: &str) -> bool {
    let main = format!(
        "#import \"/mitex/specs/mod.typ\": mitex-scope\n#math.equation(eval(\"$\" + {} + \"$\", scope: mitex-scope))",
        TypstNote::string(code)
    );
    typst::compile::<PagedDocument>(&Export::new(main, HashMap::new(), String::new(), HashMap::new())).output.is_ok()
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

/// What Typst reads: the page, the template, MiTeX's scope, the pictures,
/// the diagrams and the fonts.
struct Export {
    library: LazyHash<Library>,
    book: LazyHash<FontBook>,
    fonts: Vec<Font>,
    main: FileId,
    sources: HashMap<FileId, Source>,
    files: HashMap<String, PathBuf>,
    drawings: HashMap<String, Bytes>,
}

impl Export {
    fn new(main: String, files: HashMap<String, PathBuf>, extras: String, drawings: HashMap<String, Bytes>) -> Export {
        let fonts: Vec<Font> = FONTS.into_iter().flat_map(|data| Font::iter(Bytes::new(data))).collect();
        let id = |path: &str| FileId::new(RootedPath::new(VirtualRoot::Project, VirtualPath::new(path).expect("a path of ours")));
        let main_id = id("/main.typ");
        let sources = [("/main.typ", main), ("/export.typ", TEMPLATE.to_string()), ("/extras.typ", extras)]
            .into_iter()
            .chain(MITEX.map(|(path, text)| (path, text.to_string())))
            .map(|(path, text)| (id(path), Source::new(id(path), text)))
            .collect();
        Export {
            library: LazyHash::new(Library::default()),
            book: LazyHash::new(FontBook::from_fonts(&fonts)),
            fonts,
            main: main_id,
            sources,
            files,
            drawings,
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
        if let Some(drawing) = self.drawings.get(name) {
            return Ok(drawing.clone());
        }
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

    const DAY: chrono::NaiveDate = chrono::NaiveDate::from_ymd_opt(2026, 11, 1).unwrap();

    #[test]
    fn the_day_is_source_date_epochs_else_today() {
        assert_eq!(export_day(Some("1793836800")), chrono::NaiveDate::from_ymd_opt(2026, 11, 5).unwrap());
        assert_eq!(export_day(Some("not a number")), chrono::Local::now().date_naive());
        assert_eq!(export_day(None), chrono::Local::now().date_naive());
    }

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
        let look = Look { logo: Some(root.join("me.svg")), folder_logo: true, footer: "A footer".to_string(), day: DAY };
        run(root, &index, "Companies/Acme/Platform/Queue.md", &output, &look).unwrap();
        let pdf = fs::read(&output).unwrap();
        assert!(pdf.starts_with(b"%PDF-"));
        // No date in the file: the same note makes the same PDF.
        run(root, &index, "Companies/Acme/Platform/Queue.md", &root.join("out/again.pdf"), &look).unwrap();
        assert_eq!(fs::read(root.join("out/again.pdf")).unwrap(), pdf);
    }

    #[test]
    fn math_and_diagrams_are_drawn_and_what_does_not_parse_stays_text() {
        let root = tempfile::tempdir().unwrap();
        let root = root.path();
        let note = "# Sums\n\nInline $\\frac{a^2}{\\sqrt{x}}$ and $\\nosuchcommand{x}$.\n\n$$\n\\sum_{i=1}^{n} i = \\frac{n(n+1)}{2}\n$$\n\n```mermaid\ngraph TD\n  A[Start] --> B{Choose}\n  B -->|yes| C[سلام]\n```\n\n```mermaid\nnot a diagram at all\n```\n";
        fs::write(root.join("Sums.md"), note).unwrap();
        let index = Vault::index(root, None);
        let look = Look { logo: None, folder_logo: false, footer: String::new(), day: DAY };
        run(root, &index, "Sums.md", &root.join("Sums.pdf"), &look).unwrap();
        let pdf = fs::read(root.join("Sums.pdf")).unwrap();
        assert!(pdf.starts_with(b"%PDF-"));
        run(root, &index, "Sums.md", &root.join("again.pdf"), &look).unwrap();
        assert_eq!(fs::read(root.join("again.pdf")).unwrap(), pdf);
        assert!(diagram("graph TD\n  A --> B").is_some_and(|svg| svg.contains("Vazirmatn") && !svg.contains(DIAGRAM_FONT)));
        assert_eq!(diagram("not a diagram at all"), None);
        assert!(readable(&mitex::convert_math("\\frac{a}{b}", None).unwrap()));
        // MiTeX writes an unknown command as a call Typst cannot make.
        let unknown = mitex::convert_math("\\nosuchcommand{x}", None);
        assert!(unknown.is_err() || unknown.is_ok_and(|code| !readable(&code)));
    }

    #[test]
    fn a_logo_that_is_not_there_is_refused() {
        let root = tempfile::tempdir().unwrap();
        let root = root.path();
        fs::write(root.join("Note.md"), "Text.\n").unwrap();
        let index = Vault::index(root, None);
        let look = Look { logo: Some(root.join("missing.svg")), folder_logo: true, footer: String::new(), day: DAY };
        let error = run(root, &index, "Note.md", &root.join("Note.pdf"), &look).unwrap_err();
        assert!(error.starts_with("export.logo: "), "{error}");
        assert!(!root.join("Note.pdf").exists());
    }
}
