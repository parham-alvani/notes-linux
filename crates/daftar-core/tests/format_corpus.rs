//! `DAFTAR_FMT_CORPUS=dir cargo test -p daftar-core --test format_corpus -- --ignored`:
//! every note under `dir/NAME` formatted and compared with Prettier's output
//! of it under `dir/NAME-expected`, for each NAME given in
//! `DAFTAR_FMT_SETS` (spaces between). The notes that differ are listed,
//! and the formatted ones written under `dir/NAME-ours`.

use std::fs;
use std::path::Path;

use daftar_core::{FormatConfig, MarkdownFormat};
use walkdir::WalkDir;

#[test]
#[ignore]
fn formats_as_prettier_does() {
    let Ok(dir) = std::env::var("DAFTAR_FMT_CORPUS") else { return };
    let dir = Path::new(&dir);
    let config = FormatConfig::parse(&fs::read_to_string(dir.join("prettierrc.json")).unwrap()).unwrap();
    let sets = std::env::var("DAFTAR_FMT_SETS").unwrap_or_else(|_| "orig".to_string());
    let mut failed = 0;
    for set in sets.split_whitespace() {
        let (input, expected, ours) = (dir.join(set), dir.join(format!("{set}-expected")), dir.join(format!("{set}-ours")));
        let _ = fs::remove_dir_all(&ours);
        let (mut total, mut differ) = (0, 0);
        for entry in WalkDir::new(&input).into_iter().filter_map(Result::ok).filter(|e| e.path().extension().is_some_and(|x| x == "md")) {
            let relative = entry.path().strip_prefix(&input).unwrap();
            let text = fs::read_to_string(entry.path()).unwrap();
            let want = fs::read_to_string(expected.join(relative)).unwrap();
            let got = MarkdownFormat::format(&text, &config);
            total += 1;
            if got != want {
                differ += 1;
                let path = ours.join(relative);
                fs::create_dir_all(path.parent().unwrap()).unwrap();
                fs::write(path, &got).unwrap();
                println!("DIFF {set}/{}", relative.display());
            }
        }
        println!("{set}: {differ} of {total} differ");
        failed += differ;
    }
    assert_eq!(failed, 0);
}
