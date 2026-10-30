//! Read aloud (Ctrl+Alt+R), as the Mac's `ReadAloud`: the note from the
//! reading cursor on, a block at a time, each in its own language, with
//! speech-dispatcher's `spd-say` (the desktop's voices) or `espeak-ng`. A
//! Persian block is given a Persian voice. The page marks the block being
//! read and moves its cursor with it; Escape on the page or the command
//! again stops. One voice for the app: a new note stops the last.

use std::cell::{Cell, RefCell};
use std::ffi::OsStr;

use gtk::{gio, glib};
use webkit6::prelude::*;

/// A program that speaks, found on `$PATH`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Engine {
    SpeechDispatcher,
    Espeak,
}

#[derive(Default)]
struct Voice {
    /// The note being read, none when the voice is quiet.
    path: RefCell<Option<String>>,
    view: RefCell<Option<glib::WeakRef<webkit6::WebView>>>,
    speaking: RefCell<Option<gio::Subprocess>>,
    engine: Cell<Option<Engine>>,
    /// Grows at each start and stop, so an old loop ends.
    run: Cell<u64>,
}

thread_local! {
    static VOICE: Voice = Voice::default();
}

pub fn is_speaking() -> bool {
    VOICE.with(|voice| voice.path.borrow().is_some())
}

/// Starts reading [path] in [view], or stops when it is already reading.
/// [text] is the note, read a paragraph at a time when the page cannot list
/// its blocks. [notice] says why it cannot speak.
pub fn toggle(path: &str, view: &webkit6::WebView, text: String, notice: impl Fn(&str) + 'static) {
    if is_speaking() {
        stop();
        return;
    }
    let Some(engine) = engine() else {
        notice("Read Aloud needs speech-dispatcher (spd-say) or espeak-ng: pacman -S speech-dispatcher espeak-ng.");
        return;
    };
    let run = VOICE.with(|voice| {
        voice.run.set(voice.run.get() + 1);
        *voice.path.borrow_mut() = Some(path.to_string());
        *voice.view.borrow_mut() = Some(view.downgrade());
        voice.engine.set(Some(engine));
        voice.run.get()
    });
    let view = view.clone();
    glib::spawn_future_local(async move {
        let listed = view
            .evaluate_javascript_future("window.daftar && daftar.speechBlocks ? JSON.stringify(daftar.speechBlocks()) : null", None, None)
            .await
            .ok()
            .map(|value| value.to_str().to_string());
        let blocks = listed.and_then(|json| blocks_from_page(&json)).unwrap_or_else(|| blocks_from_text(&text));
        if blocks.is_empty() {
            stop();
            return;
        }
        for (index, text) in blocks {
            if VOICE.with(|voice| voice.run.get()) != run {
                return;
            }
            mark(index);
            let arguments = command(engine, &text);
            let arguments: Vec<&OsStr> = arguments.iter().map(OsStr::new).collect();
            let Ok(process) =
                gio::Subprocess::newv(&arguments, gio::SubprocessFlags::STDOUT_SILENCE | gio::SubprocessFlags::STDERR_SILENCE)
            else {
                break;
            };
            VOICE.with(|voice| *voice.speaking.borrow_mut() = Some(process.clone()));
            let _ = process.wait_future().await;
            glib::timeout_future(std::time::Duration::from_millis(150)).await;
        }
        if VOICE.with(|voice| voice.run.get()) == run {
            stop();
        }
    });
}

/// The voice stops, and the page's mark goes.
pub fn stop() {
    VOICE.with(|voice| {
        voice.run.set(voice.run.get() + 1);
        if let Some(process) = voice.speaking.take() {
            process.force_exit();
        }
        // speech-dispatcher speaks on in its daemon after the client ends.
        if voice.engine.get() == Some(Engine::SpeechDispatcher) {
            let _ = gio::Subprocess::newv(&[OsStr::new("spd-say"), OsStr::new("--stop")], gio::SubprocessFlags::NONE);
        }
    });
    if VOICE.with(|voice| voice.path.take()).is_some() {
        mark(-1);
    }
}

/// The voice stops when the note it reads leaves [view]'s screen.
pub fn left(view: &webkit6::WebView, path: Option<&str>) {
    let reading = VOICE.with(|voice| {
        let same_view = voice.view.borrow().as_ref().and_then(glib::WeakRef::upgrade).is_some_and(|v| &v == view);
        same_view && voice.path.borrow().as_deref() != path
    });
    if reading && is_speaking() {
        stop();
    }
}

/// The page marks block [index] of its last `speechBlocks()`; -1 clears it.
fn mark(index: i64) {
    let view = VOICE.with(|voice| voice.view.borrow().as_ref().and_then(glib::WeakRef::upgrade));
    if let Some(view) = view {
        let script = format!("window.daftar && daftar.speaking && daftar.speaking({index}); undefined");
        view.evaluate_javascript(&script, None, None, gio::Cancellable::NONE, |_| {});
    }
}

fn engine() -> Option<Engine> {
    if glib::find_program_in_path("spd-say").is_some() {
        Some(Engine::SpeechDispatcher)
    } else if glib::find_program_in_path("espeak-ng").is_some() {
        Some(Engine::Espeak)
    } else {
        None
    }
}

/// The command that says [text], waiting until it is said, in its language.
fn command(engine: Engine, text: &str) -> Vec<String> {
    let persian = is_persian(text);
    // A text that starts with "-" is not an option.
    let text = if text.starts_with('-') { format!(" {text}") } else { text.to_string() };
    match engine {
        Engine::SpeechDispatcher => {
            let mut out = vec!["spd-say".to_string(), "--wait".into()];
            if persian {
                out.extend(["--language".into(), "fa".into()]);
            }
            out.push(text);
            out
        }
        Engine::Espeak => {
            let mut out = vec!["espeak-ng".to_string()];
            if persian {
                out.extend(["-v".into(), "fa".into()]);
            }
            out.push(text);
            out
        }
    }
}

/// Mostly Persian letters: the voice to read it with is a Persian one.
fn is_persian(text: &str) -> bool {
    let letters = text.chars().filter(|c| c.is_alphabetic());
    let (mut persian, mut all) = (0, 0);
    for letter in letters {
        all += 1;
        if ('\u{0600}'..='\u{06FF}').contains(&letter) || ('\u{FB50}'..='\u{FDFF}').contains(&letter) {
            persian += 1;
        }
    }
    all > 0 && persian * 2 >= all
}

/// The page's `[{index, text}]`.
fn blocks_from_page(json: &str) -> Option<Vec<(i64, String)>> {
    let value: serde_json::Value = serde_json::from_str(json).ok()?;
    let items = value.as_array()?;
    Some(
        items
            .iter()
            .filter_map(|item| Some((item.get("index")?.as_i64()?, item.get("text")?.as_str()?.to_string())))
            .filter(|(_, text)| !text.trim().is_empty())
            .collect(),
    )
}

/// The note's paragraphs, for a page that cannot list its blocks: no front
/// matter, no code, and the marks taken out. None is marked on the page.
fn blocks_from_text(text: &str) -> Vec<(i64, String)> {
    let mut blocks = Vec::new();
    let mut current: Vec<String> = Vec::new();
    let (mut fenced, mut front) = (false, false);
    let flush = |current: &mut Vec<String>, blocks: &mut Vec<(i64, String)>| {
        let said = current.join(" ").split_whitespace().collect::<Vec<_>>().join(" ");
        if !said.is_empty() {
            blocks.push((-1, said));
        }
        current.clear();
    };
    for (number, line) in text.lines().enumerate() {
        let trimmed = line.trim();
        if number == 0 && trimmed == "---" {
            front = true;
            continue;
        }
        if front {
            front = !(trimmed == "---" || trimmed == "...");
            continue;
        }
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            fenced = !fenced;
            flush(&mut current, &mut blocks);
            continue;
        }
        if fenced {
            continue;
        }
        if trimmed.is_empty() || trimmed.starts_with('#') {
            flush(&mut current, &mut blocks);
        }
        let plain = plain(trimmed);
        if !plain.is_empty() {
            current.push(plain);
        }
        if trimmed.starts_with('#') {
            flush(&mut current, &mut blocks);
        }
    }
    flush(&mut current, &mut blocks);
    blocks
}

/// A line without its markdown: list marks, boxes, emphasis, links as their text.
fn plain(line: &str) -> String {
    let mut text = line.trim_start_matches(['#', '>', ' ']).to_string();
    for mark in ["- [ ] ", "- [x] ", "- [X] ", "- [/] ", "- [-] ", "- ", "* ", "+ "] {
        if let Some(rest) = text.strip_prefix(mark) {
            text = rest.to_string();
            break;
        }
    }
    let mut out = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '[' if chars.peek() == Some(&'[') => {
                chars.next();
                let inner: String = chars.by_ref().take_while(|&c| c != ']').collect();
                chars.next_if_eq(&']');
                // [[target|shown]] says what it shows.
                out.push_str(inner.rsplit('|').next().unwrap_or(&inner).split('#').next().unwrap_or(""));
            }
            '*' | '_' | '`' | '=' | '~' => {}
            _ => out.push(c),
        }
    }
    out.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_block_is_said_in_its_language() {
        assert!(is_persian("سلام، امروز جلسه داریم"));
        assert!(is_persian("جلسه با Sara ساعت ۱۰"));
        assert!(!is_persian("Call Sara about the trip"));
        assert!(!is_persian("12:30"));
        assert_eq!(command(Engine::SpeechDispatcher, "سلام"), ["spd-say", "--wait", "--language", "fa", "سلام"]);
        assert_eq!(command(Engine::Espeak, "Hello"), ["espeak-ng", "Hello"]);
        assert_eq!(command(Engine::SpeechDispatcher, "-n odd").last().unwrap(), " -n odd");
    }

    #[test]
    fn the_page_lists_its_blocks() {
        assert_eq!(
            blocks_from_page(r#"[{"index":2,"text":"One"},{"index":3,"text":" "},{"index":5,"text":"Two"}]"#),
            Some(vec![(2, "One".into()), (5, "Two".into())])
        );
        assert_eq!(blocks_from_page("null"), None);
    }

    #[test]
    fn a_note_is_read_by_its_paragraphs_without_code() {
        let note =
            "---\ntags: [a]\n---\n# Plan\nFirst **line** with [[Notes/Sara|Sara]]\nsame paragraph.\n\n```\ncode\n```\n- [ ] Call `Sam`\n";
        let said: Vec<String> = blocks_from_text(note).into_iter().map(|(_, text)| text).collect();
        assert_eq!(said, ["Plan", "First line with Sara same paragraph.", "Call Sam"]);
    }
}
