//! FIRST's game manual, as FIRST posts it in HTML (Word's "filtered" HTML,
//! in the Mac's old character set), read into its sections and rules.
//!
//! The manual's own marks are what is read: `<h1>`–`<h4>` are its numbered
//! sections ("7.1 Personal Safety"); a paragraph that opens with a rule's
//! ID ("G101", with `<a name=G101>` or a `RuleNumber-…` class) begins a
//! rule, its headline in a `Headline-…` span; what follows it, up to the
//! next rule or section, is the rule's: its lettered lists, its
//! `Violation` line, its blue boxes (`BlueBox…` paragraphs) and tables.
//! Nothing before the first section (the cover, the contents) is kept,
//! nor the pictures and their captions.

/// What a paragraph of a body is, by the one letter that starts its line.
pub mod mark {
    /// Plain text.
    pub const TEXT: u8 = b'P';
    /// A blue box: FIRST's explanation or example of a rule.
    pub const BLUE: u8 = b'B';
    /// An item of a list in a blue box.
    pub const BLUE_ITEM: u8 = b'b';
    /// An item of a lettered or bulleted list.
    pub const ITEM: u8 = b'L';
    /// The penalty for breaking the rule.
    pub const VIOLATION: u8 = b'V';
    /// A row of a table, its cells joined by " · ".
    pub const ROW: u8 = b'T';
    /// A table's caption.
    pub const CAPTION: u8 = b'C';
}

#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    /// A numbered section at a depth (1 a chapter, "7 Game Rules (G)").
    Section(u8),
    Rule,
}

/// One section or rule of the manual.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub kind: Kind,
    /// "G101", or a section's number: "7.1".
    pub id: String,
    /// A rule's headline, or a section's title.
    pub title: String,
    /// Its paragraphs, one a line, each led by its `mark`.
    pub body: String,
    /// The section it is in (an index into the entries), if any.
    pub parent: Option<usize>,
}

#[derive(Debug, PartialEq)]
pub struct Manual {
    /// As the cover gives it: "2026 FIRST Robotics Competition Game Manual".
    pub title: String,
    pub entries: Vec<Entry>,
}

/// The characters 0x80–0xFF stand for in the Mac's old character set
/// ("macintosh", which Word writes FIRST's manual in).
const MAC_ROMAN: [char; 128] = [
    'Ä', 'Å', 'Ç', 'É', 'Ñ', 'Ö', 'Ü', 'á', 'à', 'â', 'ä', 'ã', 'å', 'ç', 'é', 'è', //
    'ê', 'ë', 'í', 'ì', 'î', 'ï', 'ñ', 'ó', 'ò', 'ô', 'ö', 'õ', 'ú', 'ù', 'û', 'ü', //
    '†', '°', '¢', '£', '§', '•', '¶', 'ß', '®', '©', '™', '´', '¨', '≠', 'Æ', 'Ø', //
    '∞', '±', '≤', '≥', '¥', 'µ', '∂', '∑', '∏', 'π', '∫', 'ª', 'º', 'Ω', 'æ', 'ø', //
    '¿', '¡', '¬', '√', 'ƒ', '≈', '∆', '«', '»', '…', '\u{a0}', 'À', 'Ã', 'Õ', 'Œ', 'œ', //
    '–', '—', '“', '”', '‘', '’', '÷', '◊', 'ÿ', 'Ÿ', '⁄', '€', '‹', '›', 'ﬁ', 'ﬂ', //
    '‡', '·', '‚', '„', '‰', 'Â', 'Ê', 'Á', 'Ë', 'È', 'Í', 'Î', 'Ï', 'Ì', 'Ó', 'Ô', //
    '\u{f8ff}', 'Ò', 'Ú', 'Û', 'Ù', 'ı', 'ˆ', '˜', '¯', '˘', '˙', '˚', '¸', '˝', '˛', 'ˇ',
];

/// Windows' 0x80–0x9F, where it differs from Latin-1.
const CP1252: [char; 32] = [
    '€', '\u{81}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{8d}', 'Ž', '\u{8f}', //
    '\u{90}', '‘', '’', '“', '”', '•', '–', '—', '˜', '™', 'š', '›', 'œ', '\u{9d}', 'ž', 'Ÿ',
];

/// The page's bytes as text, in the character set its `<meta>` names.
pub fn decode(bytes: &[u8]) -> String {
    let head = String::from_utf8_lossy(&bytes[..bytes.len().min(4096)]).to_ascii_lowercase();
    let charset = head.split("charset=").nth(1).map(|s| s.trim_start_matches(['"', '\'']).split(|c: char| !(c.is_ascii_alphanumeric() || c == '-')).next().unwrap_or("").to_owned()).unwrap_or_default();
    match charset.as_str() {
        "macintosh" | "x-mac-roman" | "mac" => bytes.iter().map(|&b| if b < 0x80 { b as char } else { MAC_ROMAN[(b - 0x80) as usize] }).collect(),
        "windows-1252" | "iso-8859-1" | "latin1" | "us-ascii" => bytes.iter().map(|&b| if (0x80..0xa0).contains(&b) { CP1252[(b - 0x80) as usize] } else { b as char }).collect(),
        _ => String::from_utf8_lossy(bytes).into_owned(),
    }
}

/// Text in the Mac's old character set (the tests' and the sample's
/// stand-in for Word's output): what it has no byte for is left out.
pub fn encode_mac_roman(text: &str) -> Vec<u8> {
    text.chars()
        .filter_map(|c| if c.is_ascii() { Some(c as u8) } else { MAC_ROMAN.iter().position(|&m| m == c).map(|i| 0x80 + i as u8) })
        .collect()
}

/// `&amp;`, `&nbsp;`, `&#8217;` and the like, as characters.
fn entity(name: &str) -> Option<char> {
    let named = match name {
        "nbsp" => '\u{a0}',
        "amp" => '&',
        "lt" => '<',
        "gt" => '>',
        "quot" => '"',
        "apos" => '\'',
        "shy" => '\u{ad}',
        "ndash" => '–',
        "mdash" => '—',
        "rsquo" => '’',
        "lsquo" => '‘',
        "ldquo" => '“',
        "rdquo" => '”',
        "hellip" => '…',
        "bull" => '•',
        "deg" => '°',
        "reg" => '®',
        "trade" => '™',
        "copy" => '©',
        _ => {
            let n = name.strip_prefix('#')?;
            let code = match n.strip_prefix(['x', 'X']) {
                Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                None => n.parse().ok()?,
            };
            return char::from_u32(code);
        }
    };
    Some(named)
}

/// A rule's ID at the start of `text` ("G101 …"), if it opens with one:
/// one or two capital letters, three digits, then a space.
pub fn rule_id_at_start(text: &str) -> Option<&str> {
    let b = text.as_bytes();
    let letters = b.iter().take_while(|c| c.is_ascii_uppercase()).count();
    if !(1..=2).contains(&letters) || b.len() < letters + 3 {
        return None;
    }
    let digits = b[letters..].iter().take_while(|c| c.is_ascii_digit()).count();
    if digits != 3 || b.get(letters + 3).is_some_and(|c| !c.is_ascii_whitespace()) {
        return None;
    }
    Some(&text[..letters + 3])
}

/// A rule ID as typed ("g101", "G 101", "r 203"), written as the manual
/// writes it ("G101"); None if it is not one.
pub fn normalize_id(typed: &str) -> Option<String> {
    let squeezed: String = typed.trim().chars().filter(|c| !c.is_whitespace()).collect::<String>().to_ascii_uppercase();
    let letters = squeezed.chars().take_while(|c| c.is_ascii_uppercase()).count();
    let digits = squeezed.chars().skip(letters).take_while(|c| c.is_ascii_digit()).count();
    ((1..=2).contains(&letters) && digits == 3 && squeezed.len() == letters + 3).then_some(squeezed)
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Tag {
    P,
    H(u8),
    Row,
}

/// A paragraph, heading or table row as read, before it is placed.
#[derive(Debug)]
struct Block {
    tag: Tag,
    class: String,
    text: String,
    /// Where the headline span is in `text`, for a rule's paragraph.
    headline: Option<(usize, usize)>,
    anchors: Vec<String>,
}

/// The value of an attribute in a tag's attributes (quoted or not).
fn attr<'a>(attrs: &'a str, name: &str) -> Option<&'a str> {
    let lower = attrs.to_ascii_lowercase();
    let mut from = 0;
    while let Some(at) = lower[from..].find(name) {
        let at = from + at;
        let before_ok = at == 0 || lower.as_bytes()[at - 1].is_ascii_whitespace();
        let rest = &attrs[at + name.len()..];
        let rest_trim = rest.trim_start();
        if before_ok && rest_trim.starts_with('=') {
            let v = rest_trim[1..].trim_start();
            return Some(match v.as_bytes().first() {
                Some(q @ (b'"' | b'\'')) => {
                    let q = *q as char;
                    let v = &v[1..];
                    &v[..v.find(q).unwrap_or(v.len())]
                }
                _ => &v[..v.find(|c: char| c.is_ascii_whitespace() || c == '>').unwrap_or(v.len())],
            });
        }
        from = at + name.len();
    }
    None
}

/// What the handheld's fonts have no glyph for, as words they do: the
/// manual's Greek letters, fractions and ohms.
fn readable(c: char) -> Option<&'static str> {
    Some(match c {
        'Ω' => " ohm",
        'α' => "alpha",
        'β' => "beta",
        'σ' => "sigma",
        'μ' => "µ",
        'π' => "pi",
        'Δ' => "delta",
        'θ' => "theta",
        '∞' => "infinity",
        '⁄' => "/",
        '⅛' => "1/8",
        '⅜' => "3/8",
        '⅝' => "5/8",
        '⅞' => "7/8",
        '′' => "'",
        '″' => "\"",
        '†' | '‡' => "*",
        '✓' => "yes",
        '\u{f8ff}' => "",
        _ => return None,
    })
}

/// Text appended with its white space run together, so places in it hold.
fn push_text(out: &mut String, text: &str) {
    for c in text.chars() {
        if let Some(words) = readable(c) {
            out.push_str(words);
        } else if c.is_whitespace() || c == '\u{a0}' {
            if !out.is_empty() && !out.ends_with(' ') {
                out.push(' ');
            }
        } else if c == '\u{2011}' {
            // A hyphen that does not break, as a plain one.
            out.push('-');
        } else if c != '\u{ad}' {
            out.push(c);
        }
    }
}

fn push_raw(out: &mut String, raw: &str) {
    let mut rest = raw;
    while let Some(amp) = rest.find('&') {
        push_text(out, &rest[..amp]);
        let after = &rest[amp + 1..];
        match after.find(';').filter(|&semi| semi <= 10).and_then(|semi| entity(&after[..semi]).map(|c| (c, semi))) {
            Some((c, semi)) => {
                let mut buf = [0u8; 4];
                push_text(out, c.encode_utf8(&mut buf));
                rest = &after[semi + 1..];
            }
            None => {
                push_text(out, "&");
                rest = after;
            }
        }
    }
    push_text(out, rest);
}

/// The page's paragraphs, headings and table rows, in order.
fn blocks(html: &str) -> Vec<Block> {
    let lower_at = |at: usize| html.get(at..).unwrap_or("");
    let start = html.to_ascii_lowercase().find("<body").unwrap_or(0);
    let mut out = Vec::new();
    let mut open: Option<Block> = None;
    // Table rows: the cells so far, and the one being read.
    let mut row: Option<Vec<String>> = None;
    let mut cell: Option<String> = None;
    // Spans open in the paragraph: whether each is its headline.
    let mut spans: Vec<bool> = Vec::new();
    let mut i = start;
    while i < html.len() {
        let Some(lt) = html[i..].find('<').map(|k| i + k) else {
            break;
        };
        let text = &html[i..lt];
        if let Some(c) = cell.as_mut() {
            push_raw(c, text);
        } else if let Some(b) = open.as_mut() {
            push_raw(&mut b.text, text);
        }
        // A comment, skipped whole.
        if lower_at(lt).starts_with("<!--") {
            i = html[lt..].find("-->").map_or(html.len(), |k| lt + k + 3);
            continue;
        }
        let Some(gt) = html[lt..].find('>').map(|k| lt + k) else {
            break;
        };
        let inner = &html[lt + 1..gt];
        i = gt + 1;
        let closing = inner.starts_with('/');
        let inner = inner.trim_start_matches('/');
        let name_end = inner.find(|c: char| c.is_ascii_whitespace() || c == '/').unwrap_or(inner.len());
        let name = inner[..name_end].to_ascii_lowercase();
        let attrs = &inner[name_end..];
        match (name.as_str(), closing) {
            ("style" | "script" | "head", false) => {
                let close = format!("</{name}");
                i = html[i..].to_ascii_lowercase().find(&close).map_or(html.len(), |k| i + k);
            }
            ("tr", false) => {
                if let Some(b) = open.take() {
                    out.push(b);
                }
                row = Some(Vec::new());
            }
            ("tr", true) => {
                if let Some(c) = cell.take() {
                    row.get_or_insert_with(Vec::new).push(c);
                }
                if let Some(cells) = row.take() {
                    let cells: Vec<String> = cells.into_iter().map(|c| c.trim().to_owned()).filter(|c| !c.is_empty()).collect();
                    if !cells.is_empty() {
                        out.push(Block { tag: Tag::Row, class: String::new(), text: cells.join(" · "), headline: None, anchors: Vec::new() });
                    }
                }
            }
            ("td" | "th", false) if row.is_some() => {
                if let Some(c) = cell.take() {
                    row.as_mut().expect("in a row").push(c);
                }
                cell = Some(String::new());
            }
            ("td" | "th", true) if row.is_some() => {
                if let Some(c) = cell.take() {
                    row.as_mut().expect("in a row").push(c);
                }
            }
            ("p" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6", false) => {
                if let Some(c) = cell.as_mut() {
                    // A paragraph inside a table's cell: part of the cell.
                    push_text(c, " ");
                    continue;
                }
                if let Some(b) = open.take() {
                    out.push(b);
                }
                let tag = match name.as_bytes() {
                    [b'h', n] => Tag::H(n - b'0'),
                    _ => Tag::P,
                };
                spans.clear();
                open = Some(Block { tag, class: attr(attrs, "class").unwrap_or("").to_owned(), text: String::new(), headline: None, anchors: Vec::new() });
            }
            ("p" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6", true) => {
                if let Some(c) = cell.as_mut() {
                    push_text(c, " ");
                } else if let Some(b) = open.take() {
                    out.push(b);
                }
            }
            ("br", _) => {
                if let Some(c) = cell.as_mut() {
                    push_text(c, " ");
                } else if let Some(b) = open.as_mut() {
                    push_text(&mut b.text, " ");
                }
            }
            ("a", false) => {
                if let (Some(b), Some(n)) = (open.as_mut(), attr(attrs, "name")) {
                    b.anchors.push(n.to_owned());
                }
            }
            ("span", false) => {
                let headline = attr(attrs, "class").is_some_and(|c| c.starts_with("Headline"));
                if let Some(b) = open.as_mut().filter(|_| cell.is_none()) {
                    if headline && b.headline.is_none() {
                        b.headline = Some((b.text.len(), b.text.len()));
                    }
                    spans.push(headline);
                }
            }
            ("span", true) => {
                if let (Some(true), Some(b)) = (spans.pop(), open.as_mut()) {
                    if let Some((from, _)) = b.headline {
                        b.headline = Some((from, b.text.len()));
                    }
                }
            }
            _ => {}
        }
    }
    if let Some(b) = open.take() {
        out.push(b);
    }
    out
}

/// A heading's number and title: "7.1 Personal Safety" → ("7.1", "Personal Safety").
fn numbered(text: &str) -> (String, String) {
    let text = text.trim();
    let end = text.find(|c: char| !(c.is_ascii_digit() || c == '.')).unwrap_or(text.len());
    let number = text[..end].trim_end_matches('.');
    if number.is_empty() || !number.starts_with(|c: char| c.is_ascii_digit()) {
        return (String::new(), text.to_owned());
    }
    (number.to_owned(), text[end..].trim().to_owned())
}

/// The mark a paragraph of a body takes, by its class.
fn mark_of(block: &Block) -> u8 {
    let class = block.class.as_str();
    if block.tag == Tag::Row {
        mark::ROW
    } else if class.starts_with("BlueBox") && (class.contains("list") || class.contains("List")) {
        mark::BLUE_ITEM
    } else if class.starts_with("BlueBox") {
        mark::BLUE
    } else if class == "Violation" {
        mark::VIOLATION
    } else if class.contains("List") || class.contains("list") {
        mark::ITEM
    } else if class == "MsoCaption" {
        mark::CAPTION
    } else {
        mark::TEXT
    }
}

/// Whether a paragraph begins a rule: it opens with the rule's ID, and the
/// manual marks it as a rule (its anchor, or a rule's paragraph style).
fn rule_start(block: &Block) -> Option<String> {
    if block.tag != Tag::P {
        return None;
    }
    let id = rule_id_at_start(&block.text)?;
    let marked = block.anchors.iter().any(|a| a == id) || block.class.starts_with("RuleNumber") || block.class.starts_with("TRules");
    marked.then(|| id.to_owned())
}

/// Read the manual's sections and rules from its HTML.
pub fn parse(bytes: &[u8]) -> Result<Manual, String> {
    let html = decode(bytes);
    let blocks = blocks(&html);
    // The cover's lines down to the one that says "Manual" ("2026 FIRST®
    // Robotics Competition", "Game Manual"), before the contents.
    let cover: Vec<&str> = blocks.iter().take_while(|b| !matches!(b.tag, Tag::H(_)) && !b.class.starts_with("MsoToc")).map(|b| b.text.trim()).filter(|t| !t.is_empty() && t.len() < 80).collect();
    let title = match cover.iter().position(|t| t.contains("Manual")) {
        Some(k) => cover[k.saturating_sub(2)..=k].join(" "),
        None => "Game Manual".to_owned(),
    };
    let mut entries: Vec<Entry> = Vec::new();
    // The sections open at each depth, deepest last.
    let mut path: Vec<(u8, usize)> = Vec::new();
    let mut started = false;
    for block in &blocks {
        let text = block.text.trim();
        if let Tag::H(level) = block.tag {
            if text.is_empty() || level > 4 {
                continue;
            }
            started = true;
            while path.last().is_some_and(|(l, _)| *l >= level) {
                path.pop();
            }
            let (id, title) = numbered(text);
            entries.push(Entry { kind: Kind::Section(level), id, title, body: String::new(), parent: path.last().map(|(_, i)| *i) });
            path.push((level, entries.len() - 1));
            continue;
        }
        if !started || text.is_empty() || block.class.starts_with("MsoToc") {
            continue;
        }
        if let Some(id) = rule_start(block) {
            let (title, rest) = match block.headline {
                Some((from, to)) if to > from && from >= id.len() => (block.text[from..to].trim().to_owned(), block.text[to..].trim().to_owned()),
                _ => {
                    // No headline span: its first sentence.
                    let after = block.text[id.len()..].trim();
                    match after.find(". ") {
                        Some(end) => (after[..=end].trim().to_owned(), after[end + 1..].trim().to_owned()),
                        None => (after.to_owned(), String::new()),
                    }
                }
            };
            let mut body = String::new();
            if !rest.is_empty() {
                body.push(mark::TEXT as char);
                body.push_str(&rest);
            }
            entries.push(Entry { kind: Kind::Rule, id, title, body, parent: path.last().map(|(_, i)| *i) });
            continue;
        }
        let m = mark_of(block);
        // Pictures' captions name a picture that is not kept.
        if m == mark::CAPTION && !text.starts_with("Table") {
            continue;
        }
        let Some(entry) = entries.last_mut() else { continue };
        if !entry.body.is_empty() {
            entry.body.push('\n');
        }
        entry.body.push(m as char);
        entry.body.push_str(text);
    }
    if !entries.iter().any(|e| e.kind == Kind::Rule) {
        return Err("no rules found in it: it is not the game manual, or its layout changed".into());
    }
    Ok(Manual { title, entries })
}

/// A body's paragraphs: (mark, text).
pub fn paragraphs(body: &str) -> impl Iterator<Item = (u8, &str)> {
    body.lines().filter(|l| !l.is_empty()).map(|l| (l.as_bytes()[0], &l[1..]))
}

/// A body as plain text, for a model to read: blue boxes and violations
/// named, list items and rows on lines of their own.
pub fn plain(body: &str) -> String {
    let mut out = String::new();
    for (m, text) in paragraphs(body) {
        if !out.is_empty() {
            out.push('\n');
        }
        match m {
            mark::BLUE | mark::BLUE_ITEM => {
                out.push_str("Blue box: ");
                out.push_str(text);
            }
            _ => out.push_str(text),
        }
    }
    out
}
