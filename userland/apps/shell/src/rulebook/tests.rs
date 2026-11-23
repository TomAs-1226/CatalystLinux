//! Rulebook's tests, on the sample manual (written for them in the form of
//! FIRST's HTML manual; none of it FIRST's text).

use std::path::PathBuf;

use super::ask::{self, Passage};
use super::parse::{self, mark, Kind};
use super::store::{self, Book, Meta};
use super::{fetch, sample};

/// A folder of its own for a test's manual.
fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("catalyst-rulebook-{}-{name}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    dir
}

fn kept(name: &str) -> (PathBuf, Book) {
    let dir = temp(name);
    let manual = parse::parse(&sample::bytes()).expect("the sample reads");
    store::write(&dir, &manual, Meta { fetched: 100, checked: 100, ..Meta::default() }).expect("kept");
    let book = Book::open(&dir).expect("opens");
    (dir, book)
}

fn ids(book: &Book, entries: &[usize]) -> Vec<String> {
    entries.iter().map(|&i| book.items[i].id.clone()).collect()
}

// ---------------------------------------------------------------- parser ---

#[test]
fn the_manual_reads_into_its_sections_and_rules() {
    let m = parse::parse(&sample::bytes()).unwrap();
    assert_eq!(m.title, "2026 Sample Game Manual");
    // Nothing from the cover or the contents: the first entry is chapter 1.
    assert_eq!((m.entries[0].kind.clone(), m.entries[0].id.as_str(), m.entries[0].title.as_str()), (Kind::Section(1), "1", "Introduction"));
    let rules: Vec<&str> = m.entries.iter().filter(|e| e.kind == Kind::Rule).map(|e| e.id.as_str()).collect();
    assert_eq!(rules, ["G101", "G102", "G201", "G202", "G301", "G302", "G303", "R101", "R102", "R201", "R202", "R203", "R301", "R302", "E101", "E102"]);
    let find = |id: &str| m.entries.iter().position(|e| e.id == id).unwrap();
    let g101 = &m.entries[find("G101")];
    // The headline from its span, the asterisk (evergreen) kept; the rest its body.
    assert_eq!(g101.title, "*Stay outside the FIELD.");
    let paras: Vec<(u8, &str)> = parse::paragraphs(&g101.body).collect();
    assert_eq!(paras[0], (mark::TEXT, "A team member may not reach into the FIELD with any part of their body during a MATCH."));
    assert_eq!(paras[1], (mark::VIOLATION, "Violation: VERBAL WARNING. YELLOW CARD for each one after it at the event."));
    assert_eq!(paras[2].0, mark::BLUE);
    // The Mac's character set: its curly apostrophe, read as one.
    assert!(paras[2].1.contains("team’s first"), "{}", paras[2].1);
    // Sections nest: G101 in 7.1, 7.1 in chapter 7.
    let s71 = find("7.1");
    assert_eq!(g101.parent, Some(s71));
    assert_eq!(m.entries[s71].parent, Some(find("7")));
    // A lettered list keeps its letters, each item a line of its own.
    let items: Vec<&str> = parse::paragraphs(&m.entries[find("G201")].body).filter(|(k, _)| *k == mark::ITEM).map(|(_, t)| t).collect();
    assert_eq!(items, ["A. taunting another team or its ROBOT,", "B. arguing with a REFEREE about a call,", "C. leaving trash in the stands or the pit."]);
    // A table: its caption, then each row with its cells; a picture's caption is left out.
    let scoring: Vec<(u8, &str)> = parse::paragraphs(&m.entries[find("6.2")].body).collect();
    assert!(scoring.contains(&(mark::CAPTION, "Table 6-1 Points for each task")), "{scoring:?}");
    assert!(scoring.contains(&(mark::ROW, "GAME PIECE in the high GOAL · 6 · 4")), "{scoring:?}");
    assert!(!scoring.iter().any(|(_, t)| t.starts_with("Figure")));
    // A list in a blue box is the blue box's.
    assert!(parse::paragraphs(&m.entries[find("R202")].body).any(|(k, _)| k == mark::BLUE));
    // What the handheld's fonts lack is put in words they have.
    assert!(m.entries[find("R202")].body.contains("A. a backing of 3/4 in. plywood,"));
}

#[test]
fn a_page_that_is_not_the_manual_is_refused() {
    assert!(parse::parse(b"<html><body><h1>1 Hello</h1><p>No rules here.</p></body></html>").is_err());
}

#[test]
fn the_character_sets_word_writes_are_read() {
    assert_eq!(parse::decode(&[b'<', b'm', b'e', b't', b'a', b' ', b'c', b'h', b'a', b'r', b's', b'e', b't', b'=', b'm', b'a', b'c', b'i', b'n', b't', b'o', b's', b'h', b'>', 0xD5, 0xD2, 0xD3, 0xD0, 0xB2]), "<meta charset=macintosh>’“”–≤");
    assert_eq!(parse::decode(b"<meta charset=windows-1252>\x92\x96"), "<meta charset=windows-1252>’–");
    assert_eq!(parse::decode("<meta charset=utf-8>’".as_bytes()), "<meta charset=utf-8>’");
    assert_eq!(parse::encode_mac_roman("it’s ≤"), vec![b'i', b't', 0xD5, b's', b' ', 0xB2]);
}

// ------------------------------------------------------------ rule's ID ---

#[test]
fn a_rule_is_found_by_its_id_however_it_is_typed() {
    let (_dir, book) = kept("jump");
    for typed in ["G101", "g101", " g 101 ", "G101\n"] {
        assert_eq!(book.find_id(typed).map(|i| book.items[i].id.clone()).as_deref(), Some("G101"), "{typed:?}");
    }
    assert_eq!(book.find_id("G1011"), None);
    assert_eq!(book.find_id("G999"), None);
    assert_eq!(book.find_id("101"), None);
    assert_eq!(parse::normalize_id("gr 203").as_deref(), Some("GR203"));
    assert_eq!(parse::rule_id_at_start("R203 Team numbers"), Some("R203"));
    assert_eq!(parse::rule_id_at_start("R2034 x"), None);
    assert_eq!(parse::rule_id_at_start("ROBOT rules"), None);
    // Its ID typed into search: that rule first, before anything mentioning it.
    let found = book.search("G302", 10);
    assert_eq!(book.items[found.hits[0].entry].id, "G302");
    // The start of one: the rules it begins, in order.
    assert_eq!(ids(&book, &book.search("R2", 10).hits.iter().map(|h| h.entry).collect::<Vec<_>>())[..3], ["R201", "R202", "R203"]);
    // Kept text is read back from the card as written.
    let g102 = book.find_id("g102").unwrap();
    assert!(book.body(g102).contains("FIELD’s lights are green"));
}

// ---------------------------------------------------------------- search ---

#[test]
fn search_ranks_a_headline_above_a_mention_and_more_words_above_fewer() {
    let (_dir, book) = kept("rank");
    let top = |q: &str| book.items[book.search(q, 10).hits[0].entry].id.clone();
    // "pinning" is G302's headline ("No pinning"); G301's blue box only mentions pinning.
    assert_eq!(top("pinning "), "G302");
    // Both words in E101; "safety" alone is in several places.
    assert_eq!(top("safety glasses "), "E101");
    // A plural finds the singular.
    assert_eq!(top("batteries "), "R301");
    // The last word, still being typed, matches as a prefix.
    let typing = book.search("bump", 10);
    assert!(typing.hits.iter().any(|h| book.items[h.entry].id == "R201"), "{:?}", ids(&book, &typing.hits.iter().map(|h| h.entry).collect::<Vec<_>>()));
    assert!(typing.terms.iter().any(|t| t == "bumper"), "{:?}", typing.terms);
    // Nothing found for words the manual does not have.
    assert!(book.search("xylophone ", 10).hits.is_empty());
    // The matches marked in a line around the first of them.
    let found = book.search("noodles", 10);
    let r202 = book.find_id("R202").unwrap();
    assert_eq!(found.hits[0].entry, r202);
    let (text, marks) = store::snippet(&book.body(r202), &found.terms, 60);
    let marked: Vec<&str> = marks.iter().map(|&(a, b)| &text[a..b]).collect();
    assert_eq!(marked, ["noodles"], "{text}");
}

#[test]
fn search_reads_only_the_postings_it_needs() {
    let (dir, book) = kept("lazy");
    // The index is not read until the first search; its dictionary is small.
    let index = std::fs::metadata(dir.join("index.bin")).unwrap().len();
    let text = std::fs::metadata(dir.join("text.txt")).unwrap().len();
    assert!(index < text * 2, "index {index} B for {text} B of text");
    assert!(!book.search("glasses", 5).hits.is_empty());
}

// ------------------------------------------------------------------- ask ---

#[test]
fn ask_finds_the_rules_a_question_is_about() {
    let (_dir, book) = kept("retrieve");
    let found = ask::retrieve(&book, "Can a student reach into the field during a match?", 4);
    assert_eq!(book.items[found[0]].id, "G101", "{:?}", ids(&book, &found));
    // A rule named in the question comes first, whatever else matches.
    let found = ask::retrieve(&book, "what does R301 say about the main breaker", 4);
    assert_eq!(ids(&book, &found)[..2], ["R301", "R302"]);
    assert!(found.len() <= 4);
    // The passages carry the rule's words, blue boxes named.
    let passages = ask::passages(&book, &ask::retrieve(&book, "reach into the field during a match", 1));
    assert_eq!(passages[0].cite, "[G101]");
    assert!(passages[0].text.contains("Blue box: A violation counts against the whole team"));
}

#[test]
fn the_prompt_for_the_model_on_this_handheld_fits_its_context() {
    let long = "The ROBOT must not exceed the FIELD's limits during a MATCH, as the REFEREES judge it. ".repeat(60);
    let passages: Vec<Passage> = (0..5).map(|k| Passage { entry: k, cite: format!("[G10{k}]"), title: format!("Rule {k}."), text: long.clone() }).collect();
    let question = "Can we extend past the frame perimeter during the endgame?";
    for context in [2048u32, 1024, 600] {
        let (system, user) = ask::local_prompt(&passages, question, context, ask::LOCAL_ANSWER);
        let used = ask::tokens(&system) + ask::tokens(&user) + ask::LOCAL_ANSWER as usize + 48;
        assert!(used <= context as usize, "{used} tokens of {context}");
        assert!(user.ends_with(&format!("Question: {question}")));
        if context >= 1024 {
            // The best rule is given, with some of its words.
            assert!(user.contains("[G100] Rule 0.\nThe ROBOT must"), "{user}");
        }
    }
    // A short rule goes in whole.
    let short = [Passage { entry: 0, cite: "[E101]".into(), title: "Safety glasses.".into(), text: "Everyone wears safety glasses.".into() }];
    let (_, user) = ask::local_prompt(&short, "glasses?", 2048, ask::LOCAL_ANSWER);
    assert_eq!(user, "Rules:\n\n[E101] Safety glasses.\nEveryone wears safety glasses.\n\nQuestion: glasses?");
    // Claude is given the rules whole (up to a few pages each).
    let (system, user) = ask::claude_prompt(&short, "glasses?");
    assert!(system.contains("[G101]"));
    assert!(user.contains("Everyone wears safety glasses."));
}

#[test]
fn an_answer_cites_only_rules_that_are_in_the_manual() {
    let (_dir, book) = kept("cite");
    let cited = ask::cited(&book, "No [G101]; see G102 and the periods [6.1]. Also [G999] and R2034.");
    assert_eq!(ids(&book, &cited), ["G101", "G102", "6.1"]);
    // The simulator's answer cites what it was given.
    let passages = ask::passages(&book, &ask::retrieve(&book, "reach into the field during a match", 2));
    let words = ask::scripted(&passages);
    assert!(words.contains("[G101]"), "{words}");
    assert_eq!(ask::cited(&book, &words)[0], book.find_id("G101").unwrap());
}

// --------------------------------------------------------------- updates ---

#[test]
fn an_update_says_which_rules_changed() {
    let dir = temp("update");
    let p = fetch::Progress::default();
    let first = fetch::keep(&dir, &sample::bytes(), Meta { fetched: 100, ..Meta::default() }, &p).unwrap();
    assert!(matches!(first, fetch::Outcome::Kept(Meta { changes: None, .. })));
    let fetch::Outcome::Kept(meta) = fetch::keep(&dir, &sample::revised(), Meta { fetched: 200, ..Meta::default() }, &p).unwrap() else { panic!("kept") };
    let c = meta.changes.expect("changes");
    assert_eq!((c.changed, c.added, c.removed, c.at), (vec!["R301".to_owned()], vec!["G304".to_owned()], vec!["G303".to_owned()], 200));
    // Kept with the manual, for its page.
    let book = Book::open(&dir).unwrap();
    assert_eq!(book.meta.changes.as_ref().map(|c| c.count()), Some(3));
    assert!(book.find_id("G304").is_some() && book.find_id("G303").is_none());
    // The server's headers, after a redirect: the last response's.
    let head = "HTTP/1.1 301 Moved\r\nLocation: x\r\nLast-Modified: old\r\n\r\nHTTP/1.1 200 OK\r\nContent-Length: 1211124\r\nLast-Modified: Wed, 22 Apr 2026 00:08:20 GMT\r\nETag: \"0x8DE\"\r\n";
    assert_eq!(fetch::headers(head), (Some("Wed, 22 Apr 2026 00:08:20 GMT".into()), Some("\"0x8DE\"".into()), Some(1_211_124)));
    assert_eq!(fetch::short_date("Wed, 22 Apr 2026 00:08:20 GMT").as_deref(), Some("apr 22"));
}

// ------------------------------------------------------------------ rest ---

#[test]
fn at_rest_the_pages_ask_for_no_frames() {
    use crate::shell::Screen;
    crate::shell::tests::sandbox();
    let (_dir, book) = kept("rest");
    let book = std::rc::Rc::new(book);
    let page = super::Rulebook::waiting();
    assert!(page.ambient().is_none() && !page.busy());
    let search = super::Searching::new(book.clone(), "bumper");
    assert!(search.ambient().is_none() && !search.busy());
    let reader = super::Reader::new(book.clone(), vec![0], 0, Vec::new());
    assert!(reader.ambient().is_none() && !reader.busy());
    // While a download runs, it is looked at a few times a second, not every frame.
    let mut downloading = super::Rulebook::waiting();
    downloading.job = Some(super::Job::still(0.5));
    assert_eq!(downloading.ambient(), Some(super::POLL));
    assert!(!downloading.busy());
}

/// FIRST's manual downloaded from FIRST, as the handheld does, then looked
/// at again: the second look finds it current. Not run by default: it needs
/// the internet. With `RULEBOOK_REAL_SHOTS`, a few pages drawn from it there
/// (never into the repository: the text is FIRST's).
#[test]
#[ignore]
fn the_real_manual_downloads_and_is_then_current() {
    std::env::set_var("CATALYST_RULEBOOK_LIVE", "1");
    let dir = temp("live");
    let p = fetch::Progress::default();
    let first = fetch::fetch(&dir, false, &p).expect("downloaded");
    let fetch::Outcome::Kept(meta) = first else { panic!("kept") };
    println!("{} · {:?} · {:?} · {} B", meta.title, meta.modified, meta.etag, p.bytes.load(std::sync::atomic::Ordering::Relaxed));
    assert_eq!(fetch::fetch(&dir, false, &p).expect("looked at again"), fetch::Outcome::Current);
    let Some(out) = std::env::var_os("RULEBOOK_REAL_SHOTS") else { return };
    let out = PathBuf::from(out);
    std::fs::create_dir_all(&out).unwrap();
    crate::shell::tests::sandbox();
    let book = std::rc::Rc::new(Book::open(&dir).unwrap());
    for size in [crate::gfx::TextSize::Compact, crate::gfx::TextSize::Large] {
        let mut shell = crate::fullheight::shell("bezel", size);
        let save = |shell: &mut crate::shell::Shell, name: &str| {
            crate::fullheight::settle(shell, 90);
            shell.draw();
            crate::platform::sim::save_png(&shell.gfx.canvas, &out.join(format!("{name}-{}.png", size.name())), 2);
        };
        let rules: Vec<usize> = (0..book.items.len()).filter(|&i| book.items[i].is_rule()).collect();
        for id in ["G101", "R501", "G407"] {
            if let Some(at) = rules.iter().position(|&i| book.items[i].id == id) {
                shell.show(Box::new(super::Reader::new(book.clone(), rules.clone(), at, Vec::new())));
                save(&mut shell, id);
            }
        }
        shell.show(Box::new(super::Entries::chapter(book.clone(), book.chapters()[6])));
        save(&mut shell, "chapter");
        shell.show(Box::new(super::Searching::new(book.clone(), "bumper height")));
        save(&mut shell, "search");
    }
}

/// FIRST's own manual, if one is given (`RULEBOOK_REAL_HTML`): read it, and
/// say what was found. Not run by default: the manual is not in the repository.
#[test]
#[ignore]
fn the_real_manual_reads() {
    let Some(path) = std::env::var_os("RULEBOOK_REAL_HTML") else { return };
    let bytes = std::fs::read(path).unwrap();
    let started = std::time::Instant::now();
    let m = parse::parse(&bytes).unwrap();
    let took = started.elapsed();
    let rules = m.entries.iter().filter(|e| e.kind == Kind::Rule).count();
    let sections = m.entries.len() - rules;
    let text: usize = m.entries.iter().map(|e| e.body.len()).sum();
    println!("{}: {rules} rules, {sections} sections, {text} B of text, read in {took:?}", m.title);
    let dir = temp("real");
    let started = std::time::Instant::now();
    store::write(&dir, &m, Meta::default()).unwrap();
    println!("kept in {:?}", started.elapsed());
    for f in ["manual.json", "text.txt", "index.bin"] {
        println!("{f}: {} B", std::fs::metadata(dir.join(f)).unwrap().len());
    }
    let book = Book::open(&dir).unwrap();
    for q in ["G101", "bumper height ", "fuel in the hub ", "how many batteries ", "R2"] {
        let started = std::time::Instant::now();
        let found = book.search(q, 5);
        println!("{q:?} in {:?}: {:?}", started.elapsed(), found.hits.iter().map(|h| book.items[h.entry].label()).collect::<Vec<_>>());
    }
    for id in ["G101", "R101", "E101", "T201", "Q101", "I101"] {
        let i = book.find_id(id).unwrap_or_else(|| panic!("{id}"));
        println!("{id}: {} | {}", book.items[i].title, book.body(i).lines().next().unwrap_or("").chars().take(90).collect::<String>());
    }
    let mut seen = std::collections::BTreeSet::new();
    for e in m.entries.iter().filter(|e| e.kind == Kind::Rule) {
        assert!(seen.insert(e.id.clone()), "{} twice", e.id);
        assert!(!e.title.is_empty(), "{} has no headline", e.id);
    }
    assert!(rules > 150, "{rules} rules");
}
