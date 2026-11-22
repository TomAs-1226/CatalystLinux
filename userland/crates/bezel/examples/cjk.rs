//! Chinese, Japanese and Korean beside Google Sans: a panel of mixed lines at
//! the shell's three text sizes, written to a PNG (upscaled for inspection),
//! and, on Linux, what drawing them cost the process in memory.
//! `cargo run -p bezel --example cjk --release -- out.png [light]`

use bezel::*;

/// The process's resident memory, in kB: all of it, and its anonymous and
/// file-backed parts (Linux only).
fn rss() -> Option<(u64, u64, u64)> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let field = |name: &str| -> Option<u64> { status.lines().find(|l| l.starts_with(name))?.split_whitespace().nth(1)?.parse().ok() };
    Some((field("VmRSS:")?, field("RssAnon:")?, field("RssFile:")?))
}

/// How much of the CJK font's mapping is resident, and how much of that is
/// locked into memory, in kB (Linux only).
fn font_rss() -> Option<(u64, u64)> {
    let smaps = std::fs::read_to_string("/proc/self/smaps").ok()?;
    let mut inside = false;
    let (mut rss, mut locked) = (0, 0);
    for line in smaps.lines() {
        // A mapping's first line: "start-end perms offset dev inode [path]".
        if line.split_whitespace().next().is_some_and(|f| f.contains('-') && f.chars().all(|c| c.is_ascii_hexdigit() || c == '-')) {
            inside = line.ends_with(bezel::text::CJK_FILE);
        }
        if inside {
            let kb = |v: &str| v.split_whitespace().next().and_then(|n| n.parse::<u64>().ok()).unwrap_or(0);
            if let Some(v) = line.strip_prefix("Rss:") {
                rss += kb(v);
            } else if let Some(v) = line.strip_prefix("Locked:") {
                locked += kb(v);
            }
        }
    }
    Some((rss, locked))
}

fn report(what: &str) {
    if let Some((all, anon, file)) = rss() {
        let (font, locked) = font_rss().unwrap_or_default();
        println!("{what:<34} VmRSS {all:>6} kB  (anonymous {anon:>6}, file {file:>6}; of the font {font:>5}, locked {locked})");
    }
}

/// The lines shown: what Music, Files, Notes and Spotlight meet.
const LINES: &[&str] = &[
    "夜に駆ける · YOASOBI",
    "晴天 — 周杰倫 (Jay Chou)",
    "千と千尋の神隠し OST 2001",
    "我们的歌 · 告白气球",
    "레드벨벳 Psycho 2019",
    "ノート「明日の練習」",
    "Привет, мир · ελληνικά",
];

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let out = args.get(1).cloned().unwrap_or_else(|| "cjk.png".into());
    let tokens = if args.iter().any(|a| a == "light") { Tokens::light() } else { Tokens::dark() };
    let (w, h) = (320, 170);
    let mut canvas = Canvas::new(w, h);
    let mut text = Text::new();

    // As the shell does on the handheld: its memory locked as it is touched.
    // The font's pages must stay out of that.
    #[cfg(target_os = "linux")]
    if args.iter().any(|a| a == "lock") {
        // SAFETY: a plain system call, no memory handed over.
        let locked = unsafe { libc::mlockall(libc::MCL_CURRENT | libc::MCL_FUTURE | libc::MCL_ONFAULT) };
        println!("mlockall(CURRENT | FUTURE | ONFAULT): {}", if locked == 0 { "on".to_owned() } else { std::io::Error::last_os_error().to_string() });
    }

    report("started");
    canvas.clear(tokens.ground);
    // Latin first: the CJK font is not touched.
    for (i, size) in [12.0, 13.0, 15.0].iter().enumerate() {
        text.draw(&mut canvas, 8.0, 14.0 + i as f32 * 16.0, "Harbour Lights · Wren & the Tide 3:34", &TextStyle::body(*size), tokens.ink, 1.0);
    }
    assert!(!text.cjk_loaded(), "Latin alone must not load the CJK font");
    report("after a panel of Latin");

    canvas.clear(tokens.ground);
    // The three text sizes' title, body and caption, as gfx::Type has them.
    let sizes = [("compact", 15.0, 12.0, 10.0), ("standard", 16.0, 13.0, 11.0), ("large", 18.0, 15.0, 13.0)];
    let mut y = 4.0;
    for (k, (_, title, body, caption)) in sizes.iter().enumerate() {
        y += title + 2.0;
        text.draw(&mut canvas, 8.0, y, LINES[k * 2 % LINES.len()], &TextStyle::title(*title), tokens.ink, 1.0);
        y += body + 3.0;
        text.draw(&mut canvas, 8.0, y, LINES[(k * 2 + 1) % LINES.len()], &TextStyle::body(*body), tokens.dim, 1.0);
        y += caption + 3.0;
        text.draw(&mut canvas, 8.0, y, "ファイル 名前.txt · 2.4 kb", &TextStyle::label(*caption), tokens.dim, 1.0);
    }
    text.draw(&mut canvas, 170.0, 20.0, "레드벨벳 Psycho", &TextStyle::name(15.0), tokens.signal, 1.0);
    text.draw(&mut canvas, 170.0, 40.0, "Привет, мир", &TextStyle::body(13.0), tokens.ink, 1.0);
    text.draw(&mut canvas, 170.0, 76.0, "9:41 晴", &TextStyle::display(30.0), tokens.ink, 1.0);
    // On one baseline: capitals, x-height, descenders and ideographs side by side.
    canvas.fill_rect(Rect::new(170.0, 104.0, 145.0, 1.0), tokens.signal, 0.5);
    text.draw(&mut canvas, 170.0, 104.0, "Hxg漢字あア한 Ay", &TextStyle::body(16.0), tokens.ink, 1.0);
    canvas.fill_rect(Rect::new(170.0, 128.0, 145.0, 1.0), tokens.signal, 0.5);
    text.draw(&mut canvas, 170.0, 128.0, "ab 漢字あ cd 01", &TextStyle::label(13.0), tokens.ink, 1.0);
    report("after a panel of CJK");
    // A screen full of different characters: every one a new glyph.
    let mut scratch = Canvas::new(w, h);
    let han: String = (0x4E00u32..0x4E00 + 3000).filter_map(char::from_u32).collect();
    let chars: Vec<char> = han.chars().collect();
    for (row, line) in chars.chunks(24).enumerate() {
        let s: String = line.iter().collect();
        text.draw(&mut scratch, 4.0, 14.0 + (row % 10) as f32 * 16.0, &s, &TextStyle::body(13.0), tokens.ink, 1.0);
    }
    let (glyphs, bytes) = text.cached();
    println!("3000 different ideographs drawn: {glyphs} glyphs, {} kB cached (at most {} and {} kB)", bytes / 1024, Text::CACHE_MOST.0, Text::CACHE_MOST.1 / 1024);
    report("after 3000 different ideographs");

    let scale = 3usize;
    let (ow, oh) = (w as usize * scale, h as usize * scale);
    let mut rgb = vec![0u8; ow * oh * 3];
    for y in 0..oh {
        for x in 0..ow {
            let p = canvas.px[(y / scale) * w as usize + x / scale];
            let o = (y * ow + x) * 3;
            rgb[o] = (p >> 16) as u8;
            rgb[o + 1] = (p >> 8) as u8;
            rgb[o + 2] = p as u8;
        }
    }
    let file = std::fs::File::create(&out).unwrap();
    let mut enc = png::Encoder::new(file, ow as u32, oh as u32);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header().unwrap().write_image_data(&rgb).unwrap();
    println!("wrote {out}");
}
