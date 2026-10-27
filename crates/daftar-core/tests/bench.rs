//! `cargo test --release -p daftar-core --test bench -- --ignored --nocapture`:
//! the paths a person waits on, timed on a made-up vault of 20,000 notes, as
//! the Mac's `just bench` times them. Off in a plain `cargo test`.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::time::Instant;

use daftar_core::{Day, QueryTask, SearchIndex, TasksQuery, Vault, VaultMoves};

/// SplitMix64, from a fixed seed.
struct Seeded(u64);

impl Seeded {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
    fn chance(&mut self, percent: usize) -> bool {
        self.below(100) < percent
    }
    fn pick<'a>(&mut self, items: &'a [&'a str]) -> &'a str {
        items[self.below(items.len())]
    }
}

const WORDS: &[&str] = &[
    "plan",
    "meeting",
    "budget",
    "review",
    "design",
    "server",
    "cluster",
    "deploy",
    "network",
    "invoice",
    "contract",
    "travel",
    "family",
    "garden",
    "book",
    "chapter",
    "reading",
    "summary",
    "health",
    "doctor",
    "bank",
    "report",
    "weekly",
    "goal",
    "idea",
    "draft",
    "research",
    "model",
    "data",
    "backup",
    "kubernetes",
    "database",
    "postgres",
    "redis",
    "kafka",
    "release",
    "bug",
    "feature",
    "customer",
    "partner",
    "office",
    "team",
    "project",
    "roadmap",
    "launch",
    "coffee",
    "bakery",
    "flight",
    "hotel",
];
const PERSIAN: &[&str] = &["سلام", "دنیا", "یادداشت", "جلسه", "پروژه", "برنامه", "کتاب", "خانه", "سفر", "کار"];

fn write_vault(root: &Path, count: usize) -> Vec<String> {
    let mut random = Seeded(20_261_026);
    let mut folders = Vec::new();
    for top in ["Projects", "Areas", "Companies", "People", "Journal", "Resources", "Archive", "Meetings", "Reading", "Inbox"] {
        for a in 0..6 {
            for b in 0..5 {
                folders.push(format!("{top}/{} {a}/{} {b}", random.pick(WORDS), random.pick(WORDS)));
            }
        }
    }
    let mut titles = Vec::new();
    let mut notes = Vec::new();
    for number in 0..count {
        let title = if number < 20 {
            format!("Hub {number}")
        } else {
            let n = 2 + random.below(3);
            format!("{} {number}", (0..n).map(|_| random.pick(WORDS)).collect::<Vec<_>>().join(" "))
        };
        notes.push(format!("{}/{title}.md", folders[random.below(folders.len())]));
        titles.push(title);
    }
    for folder in &folders {
        fs::create_dir_all(root.join(folder)).unwrap();
    }
    for (number, path) in notes.iter().enumerate() {
        let mut text = String::new();
        if random.chance(30) {
            text.push_str(&format!("---\ntags: [project]\naliases: [\"{} alias\"]\n---\n", titles[number]));
        }
        text.push_str(&format!("# {}\n\n", titles[number]));
        for _ in 0..(5 + random.below(25)) {
            let mut line: Vec<String> = (0..(6 + random.below(12)))
                .map(|_| if random.chance(10) { random.pick(PERSIAN).to_string() } else { random.pick(WORDS).to_string() })
                .collect();
            if random.chance(35) {
                let target = if random.chance(3) { &titles[0] } else { &titles[random.below(titles.len())] };
                line.push(format!("[[{target}]]"));
            }
            if random.chance(8) {
                line.push("#idea".into());
            }
            if random.chance(15) {
                let status = [" ", " ", "x", "/"][random.below(4)];
                let mut task = format!("- [{status}] {}", line.join(" "));
                if random.chance(40) {
                    task.push_str(&format!(" 📅 2026-{:02}-{:02}", 1 + random.below(12), 1 + random.below(28)));
                }
                text.push_str(&task);
            } else {
                text.push_str(&line.join(" "));
            }
            text.push_str("\n\n");
        }
        if random.chance(5) {
            text.push_str("```rust\nfn main() {}\n```\n");
        }
        fs::write(root.join(path), text).unwrap();
    }
    notes
}

fn time<T>(label: &str, work: impl FnOnce() -> T) -> T {
    let start = Instant::now();
    let out = work();
    println!("{label:<28} {:>8.1} ms", start.elapsed().as_secs_f64() * 1000.0);
    out
}

#[test]
#[ignore]
fn twenty_thousand_notes() {
    let dir = tempfile::tempdir().unwrap();
    let notes = time("write the vault", || write_vault(dir.path(), 20_000));
    let index = time("index", || Vault::index(dir.path(), None));
    let search = SearchIndex::new();
    time("index with search", || Vault::index(dir.path(), Some(&search)));
    let mut again = index.clone();
    time("derive, nothing changed", || again.derive());
    let query = TasksQuery::parse("not done\ndue before in two weeks\ngroup by filename\nsort by due", None);
    let found = time("tasks query", || query.run(index.tasks(), Day::today(), |t| QueryTask::from(t)));
    println!("  {} tasks", found.len());
    let hub = notes[0].clone();
    let moves = BTreeMap::from([(hub.clone(), "Inbox/Hub renamed.md".to_string())]);
    let root = dir.path().to_path_buf();
    let changed = time("relink the hub", || VaultMoves::relinked(&index, &moves, |p| fs::read_to_string(root.join(p)).ok()));
    println!("  {} notes relinked", changed.len());
    time("search", || search.search("deploy kafka", 100));
}
