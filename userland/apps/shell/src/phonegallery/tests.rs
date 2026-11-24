use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use chrono::NaiveDateTime;

use super::serve::{self, route, span, Body, Reply, Shared, Span};
use super::*;
use crate::lan::Request;
use crate::shell::Screen;

fn look() -> Look {
    Look { accent: bezel::Accent::Rose, name: "catalyst-5805".into(), h24: true }
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("catalyst-gallery-{name}-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A real JPEG, small: a gradient the decoder reads at an eighth.
fn jpeg(path: &Path, w: u16, h: u16) {
    let mut rgb = Vec::with_capacity(w as usize * h as usize * 3);
    for y in 0..h {
        for x in 0..w {
            rgb.extend_from_slice(&[(u32::from(x) * 255 / u32::from(w)) as u8, (u32::from(y) * 255 / u32::from(h)) as u8, 120]);
        }
    }
    let mut out = Vec::new();
    jpeg_encoder::Encoder::new(&mut out, 85).encode(&rgb, w, h, jpeg_encoder::ColorType::Rgb).unwrap();
    std::fs::write(path, out).unwrap();
}

/// An album as the camera leaves one, with what lies beside its photos.
fn album(name: &str) -> PathBuf {
    let dir = scratch(name);
    jpeg(&dir.join("IMG_20261121_140312.jpg"), 640, 480);
    jpeg(&dir.join("IMG_20261120_091500.jpg"), 320, 240);
    std::fs::write(dir.join("PIX_20261121_150000.gif"), b"GIF89a\x10\x00\x10\x00\x00\x00\x00;").unwrap();
    // Not the album's: notes, a RAW, a sidecar, the originals and thumbnails.
    std::fs::write(dir.join("notes.txt"), b"secret").unwrap();
    std::fs::write(dir.join("IMG_20261121_140312.dng"), b"raw").unwrap();
    crate::photolog::meta::write(&dir.join("IMG_20261121_140312.jpg"), &crate::photolog::meta::About { caption: "Bumpers on </script><b>x</b>".into(), by: "claude-opus-5-5".into(), place: "Pit".into(), event: "Finger Lakes Regional".into(), context: "before Q32".into(), ..Default::default() }).unwrap();
    std::fs::create_dir_all(dir.join(".originals")).unwrap();
    std::fs::write(dir.join(".originals").join("IMG_20261121_140312.jpg"), b"original").unwrap();
    std::fs::create_dir_all(dir.join(".thumbs")).unwrap();
    dir
}

/// A request as a phone sends it.
fn req(method: &str, target: &str, headers: &[(&str, &str)]) -> Request {
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let mut head = format!("{method} {target} HTTP/1.1\r\nHost: 192.168.43.27\r\n");
    for (k, v) in headers {
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    head.push_str("\r\n");
    Request { method: method.into(), path: path.into(), query: query.into(), head, ..Request::default() }
}

/// In by the token: the cookie a phone then sends.
fn let_in(shared: &Shared) -> String {
    let (token, _) = shared.secrets();
    let r = route(&req("GET", &format!("/g/{token}"), &[]), shared);
    assert_eq!(r.status, "303 See Other");
    assert_eq!(r.header("Location"), Some("/"));
    let cookie = r.header("Set-Cookie").unwrap();
    assert!(cookie.contains("HttpOnly") && cookie.contains("SameSite=Lax"), "{cookie}");
    cookie.split(';').next().unwrap().to_owned()
}

fn get(shared: &Shared, cookie: &str, target: &str) -> Reply {
    route(&req("GET", target, &[("Cookie", cookie)]), shared)
}

#[test]
fn the_gallery_offers_exactly_what_the_album_lists() {
    let dir = album("listing");
    let listed: Vec<String> = crate::photos::list_in(&dir).into_iter().map(|p| p.name).collect();
    let offered: Vec<String> = items(&dir).into_iter().map(|i| i.photo.name).collect();
    assert_eq!(offered, listed, "through the album's own listing");
    assert_eq!(offered, ["PIX_20261121_150000.gif", "IMG_20261121_140312.jpg", "IMG_20261120_091500.jpg"], "newest first");
    let shared = Shared::new(dir.clone(), look(), serve::OPEN_FOR);
    let cookie = let_in(&shared);
    // Every name listed is served, as itself, with its type.
    for name in &listed {
        let r = get(&shared, &cookie, &format!("/p/{name}"));
        assert_eq!(r.status, "200 OK", "{name}");
        assert_eq!(r.kind, mime_of(name));
        let Body::File { path, from, len } = &r.body else { panic!("streamed from the card") };
        assert_eq!((path.as_path(), *from, *len), (dir.join(name).as_path(), 0, std::fs::metadata(dir.join(name)).unwrap().len()));
        assert!(r.header("Content-Disposition").unwrap().starts_with("inline;"));
        let saved = get(&shared, &cookie, &format!("/d/{name}"));
        assert!(saved.header("Content-Disposition").unwrap().starts_with("attachment; filename=\""));
    }
    // The page lists them, captions and all, and nothing else.
    let page = get(&shared, &cookie, "/");
    let html = String::from_utf8(page.bytes().to_vec()).unwrap();
    for name in &listed {
        assert!(html.contains(&format!("\"n\":\"{name}\"")), "{name}");
    }
    assert!(html.contains("before Q32") && html.contains("Pit · Finger Lakes Regional"), "match and place");
    assert!(!html.contains("notes.txt") && !html.contains(".dng") && !html.contains(".originals"));
    assert!(!html.contains("</script><b>"), "a caption cannot end the page's script");
    assert!(page.header("Content-Security-Policy").unwrap().contains("default-src 'none'"));
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn nothing_outside_the_album_is_reachable() {
    let dir = album("traversal");
    let shared = Shared::new(dir.clone(), look(), serve::OPEN_FOR);
    let cookie = let_in(&shared);
    for target in [
        "/p/notes.txt",
        "/p/IMG_20261121_140312.dng",
        "/p/IMG_20261121_140312.xmp",
        "/p/.originals%2FIMG_20261121_140312.jpg",
        "/p/.originals/IMG_20261121_140312.jpg",
        "/p/..%2F..%2Fetc%2Fpasswd",
        "/p/../../etc/passwd",
        "/p/%2e%2e",
        "/p/%2Fetc%2Fpasswd",
        "/p/IMG_20261121_140312.jpg%00.txt",
        "/p/IMG_20261121_140312.JPG",
        "/t/.thumbs",
        "/d/..%5C..%5Cnotes.txt",
        "/x/IMG_20261121_140312.jpg",
        "/p/bad%zz",
    ] {
        let r = get(&shared, &cookie, target);
        assert_eq!(r.status, "404 Not Found", "{target}");
        assert!(matches!(r.body, Body::Bytes(_)), "{target}: nothing streamed");
    }
    // Read-only: nothing is deleted or put there.
    for method in ["DELETE", "PUT", "PATCH"] {
        assert_eq!(route(&req(method, "/p/IMG_20261121_140312.jpg", &[("Cookie", &cookie)]), &shared).status, "405 Method Not Allowed");
    }
    assert_eq!(route(&req("POST", "/p/IMG_20261121_140312.jpg", &[("Cookie", &cookie)]), &shared).status, "405 Method Not Allowed");
    assert!(dir.join("IMG_20261121_140312.jpg").exists());
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn files_have_their_type_by_extension() {
    for (name, kind) in [
        ("IMG_1.jpg", "image/jpeg"),
        ("IMG_1.JPG", "image/jpeg"),
        ("a.jpeg", "image/jpeg"),
        ("PIX_1.png", "image/png"),
        ("PIX_1.gif", "image/gif"),
        ("TL_1.mp4", "video/mp4"),
        ("TL_1.MP4", "video/mp4"),
        ("TL_1.m4v", "video/mp4"),
        ("TL_1.webm", "video/webm"),
        ("TL_1.mov", "video/quicktime"),
        ("x.webp", "image/webp"),
        ("notes", "application/octet-stream"),
        ("x.exe", "application/octet-stream"),
    ] {
        assert_eq!(mime_of(name), kind, "{name}");
    }
    let photo = crate::photos::Photo { path: "TL_1.mp4".into(), name: "TL_1.mp4".into(), taken: NaiveDateTime::default(), bytes: 1 };
    let video = Item { mime: mime_of("TL_1.mp4"), photo, about: Default::default() };
    assert!(video.video() && video.moving());
}

#[test]
fn the_way_in_is_the_token_or_the_code_and_five_wrong_stop_sharing() {
    let dir = album("token");
    let shared = Shared::new(dir.clone(), look(), serve::OPEN_FOR);
    let (token, code) = (shared.secrets().0.to_owned(), shared.secrets().1.to_owned());
    assert_eq!(token.len(), 26);
    assert!(token.chars().all(|c| c.is_ascii_uppercase() || ('2'..='7').contains(&c)), "a QR code's compact alphabet");
    // Without a cookie: the way in at `/`, nothing else.
    let r = route(&req("GET", "/", &[]), &shared);
    assert_eq!(r.status, "200 OK");
    let html = String::from_utf8(r.bytes().to_vec()).unwrap();
    assert!(html.contains("six digits") && !html.contains(&token) && !html.contains(&code), "the code page gives nothing away");
    for target in ["/p/IMG_20261121_140312.jpg", "/t/IMG_20261121_140312.jpg", "/zip/all", "/list.json"] {
        assert_eq!(route(&req("GET", target, &[]), &shared).status, "403 Forbidden", "{target}");
        assert_eq!(route(&req("GET", target, &[("Cookie", "catalyst_gallery=0123456789abcdef0123456789abcdef")]), &shared).status, "403 Forbidden", "a made-up cookie: {target}");
    }
    // The QR code's link (upper case, as it is encoded), and the digits typed.
    let r = route(&req("GET", &format!("/G/{token}"), &[]), &shared);
    assert_eq!(r.status, "303 See Other");
    let typed = format!("{} {}", &code[..3], &code[3..]);
    let r = route(&req("GET", &format!("/g?c={}", typed.replace(' ', "+")), &[]), &shared);
    assert_eq!(r.status, "303 See Other", "the six digits, with the space the screen shows");
    let cookie = r.header("Set-Cookie").unwrap().split(';').next().unwrap().to_owned();
    assert_eq!(get(&shared, &cookie, "/list.json").status, "200 OK");
    // Wrong codes: four are told so; the fifth stops sharing.
    for n in 1..serve::TRIES {
        let r = route(&req("GET", &format!("/g/{}", if n % 2 == 0 { "000000" } else { "AAAAAAAAAAAAAAAAAAAAAAAAAA" }), &[]), &shared);
        assert_eq!(r.status, "403 Forbidden");
        assert!(!shared.stop.load(std::sync::atomic::Ordering::Relaxed), "{n} wrong");
    }
    let r = route(&req("GET", "/g?c=999999", &[]), &shared);
    assert!(String::from_utf8(r.bytes().to_vec()).unwrap().contains("Too many wrong codes"));
    assert!(shared.stop.load(std::sync::atomic::Ordering::Relaxed));
    // Then nothing, even with the cookie.
    assert_eq!(get(&shared, &cookie, "/p/IMG_20261121_140312.jpg").status, "410 Gone");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_page_elsewhere_pointing_its_name_here_is_refused() {
    assert!(serve::host_ok(Some("192.168.43.27")) && serve::host_ok(Some("192.168.43.27:8080")) && serve::host_ok(Some("[fe80::1]:80")));
    assert!(serve::host_ok(Some("catalyst-5805.local")) && serve::host_ok(None));
    assert!(!serve::host_ok(Some("evil.example.com")) && !serve::host_ok(Some("photos.attacker.net:80")));
    let dir = album("rebinding");
    let shared = Shared::new(dir.clone(), look(), serve::OPEN_FOR);
    let (token, _) = shared.secrets();
    let mut r = req("GET", &format!("/g/{token}"), &[]);
    r.head = r.head.replace("Host: 192.168.43.27", "Host: evil.example.com");
    assert_eq!(route(&r, &shared).status, "421 Misdirected Request");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn ranges_are_one_span_at_most() {
    assert_eq!(span(None, 100), Span::Whole);
    assert_eq!(span(Some("bytes=0-1"), 100), Span::Part(0, 1), "Safari's first look at a video");
    assert_eq!(span(Some("bytes=10-"), 100), Span::Part(10, 99));
    assert_eq!(span(Some("bytes=-30"), 100), Span::Part(70, 99));
    assert_eq!(span(Some("bytes=-300"), 100), Span::Part(0, 99));
    assert_eq!(span(Some("bytes=90-500"), 100), Span::Part(90, 99));
    assert_eq!(span(Some("bytes=100-"), 100), Span::Unsatisfiable);
    assert_eq!(span(Some("bytes=-0"), 100), Span::Unsatisfiable);
    assert_eq!(span(Some("bytes=0-1,5-9"), 100), Span::Whole, "several: the whole file");
    assert_eq!(span(Some("bytes=9-5"), 100), Span::Whole);
    assert_eq!(span(Some("items=0-5"), 100), Span::Whole);
    // As served: 206 with where it is, 416 past the end.
    let dir = album("ranges");
    let shared = Shared::new(dir.clone(), look(), serve::OPEN_FOR);
    let cookie = let_in(&shared);
    let size = std::fs::metadata(dir.join("IMG_20261121_140312.jpg")).unwrap().len();
    let r = route(&req("GET", "/p/IMG_20261121_140312.jpg", &[("Cookie", &cookie), ("Range", "bytes=100-199")]), &shared);
    assert_eq!(r.status, "206 Partial Content");
    assert_eq!(r.header("Content-Range"), Some(format!("bytes 100-199/{size}").as_str()));
    assert_eq!(r.len(), 100);
    let r = route(&req("GET", "/p/IMG_20261121_140312.jpg", &[("Cookie", &cookie), ("Range", &format!("bytes={size}-"))]), &shared);
    assert_eq!(r.status, "416 Range Not Satisfiable");
    assert_eq!(r.header("Content-Range"), Some(format!("bytes */{size}").as_str()));
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_thumbnail_is_the_albums_own_as_a_small_jpeg() {
    let dir = album("thumbs");
    let shared = Shared::new(dir.clone(), look(), serve::OPEN_FOR);
    let cookie = let_in(&shared);
    let r = get(&shared, &cookie, "/t/IMG_20261121_140312.jpg");
    assert_eq!((r.status, r.kind), ("200 OK", "image/jpeg"));
    let mut d = jpeg_decoder::Decoder::new(r.bytes());
    d.read_info().unwrap();
    let info = d.info().unwrap();
    assert_eq!((info.width as usize, info.height as usize), (crate::photos::THUMB_W, crate::photos::THUMB_H));
    assert!(dir.join(".thumbs").join("IMG_20261121_140312.jpg.thumb").is_file(), "made once, kept beside the album's");
    // One that cannot be made (not a picture this reads): none, said plainly.
    assert_eq!(get(&shared, &cookie, "/t/PIX_20261121_150000.gif").status, "404 Not Found");
    std::fs::remove_dir_all(&dir).ok();
}

/// A ZIP read back the way an unzipper reads one: the central directory,
/// then each local header, its sizes, and the CRC of what follows it.
fn unzip(bytes: &[u8]) -> Vec<(String, Vec<u8>)> {
    let u16_at = |at: usize| u16::from_le_bytes([bytes[at], bytes[at + 1]]) as u64;
    let u32_at = |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap()) as u64;
    let u64_at = |at: usize| u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap());
    let end = bytes.len() - 22;
    assert_eq!(u32_at(end), 0x0605_4b50, "the end record");
    let (mut count, mut cd) = (u16_at(end + 10), u32_at(end + 16));
    if count == 0xFFFF || cd == 0xFFFF_FFFF {
        let locator = end - 20;
        assert_eq!(u32_at(locator), 0x0706_4b50);
        let at = u64_at(locator + 8) as usize;
        assert_eq!(u32_at(at), 0x0606_4b50);
        count = u64_at(at + 32);
        cd = u64_at(at + 48);
    }
    let mut at = cd as usize;
    let mut out = Vec::new();
    for _ in 0..count {
        assert_eq!(u32_at(at), 0x0201_4b50);
        let (crc, size, nlen, xlen) = (u32_at(at + 16), u32_at(at + 24), u16_at(at + 28) as usize, u16_at(at + 30) as usize);
        assert_eq!(u16_at(at + 10), 0, "stored");
        let mut offset = u32_at(at + 42);
        let name = String::from_utf8(bytes[at + 46..at + 46 + nlen].to_vec()).unwrap();
        if offset == 0xFFFF_FFFF {
            let x = at + 46 + nlen;
            assert_eq!((u16_at(x), u16_at(x + 2)), (1, 8), "the ZIP64 extra field");
            offset = u64_at(x + 4);
        }
        let lh = offset as usize;
        assert_eq!(u32_at(lh), 0x0403_4b50);
        assert_eq!((u32_at(lh + 14), u32_at(lh + 18), u32_at(lh + 22)), (crc, size, size), "{name}: the local header has its sizes and CRC");
        let data_at = lh + 30 + u16_at(lh + 26) as usize + u16_at(lh + 28) as usize;
        let data = bytes[data_at..data_at + size as usize].to_vec();
        assert_eq!(crc32fast::hash(&data) as u64, crc, "{name}");
        out.push((name, data));
        at += 46 + nlen + xlen;
    }
    out
}

/// The archive tested by a real unzipper too, where one is to hand.
fn checked_by_others(bytes: &[u8], name: &str) {
    let path = std::env::temp_dir().join(format!("catalyst-gallery-{name}-{}.zip", std::process::id()));
    std::fs::write(&path, bytes).unwrap();
    let run = |cmd: &str, args: &[&str]| std::process::Command::new(cmd).args(args).arg(&path).output().ok();
    if let Some(out) = run("unzip", &["-t"]) {
        assert!(out.status.success(), "unzip -t: {}", String::from_utf8_lossy(&out.stdout));
        assert!(String::from_utf8_lossy(&out.stdout).contains("No errors detected"));
    }
    // A Python that runs (not a stand-in that only says how to get one).
    for python in ["python3", "python", "py"] {
        let works = std::process::Command::new(python).args(["-c", "import zipfile"]).output().is_ok_and(|o| o.status.success());
        if !works {
            continue;
        }
        let out = run(python, &["-m", "zipfile", "-t"]).unwrap();
        assert!(out.status.success(), "{python} zipfile: {}", String::from_utf8_lossy(&out.stderr));
        break;
    }
    std::fs::remove_file(&path).ok();
}

#[test]
fn a_zip_is_streamed_whole_stored_and_checks_out() {
    let dir = album("zip");
    let all = items(&dir);
    let entries: Vec<zip::Entry> = all.iter().map(|i| zip::Entry { name: i.photo.name.clone(), path: i.photo.path.clone(), size: i.photo.bytes, when: i.photo.taken }).collect();
    for (plan, name) in [(zip::Plan::new(entries.clone()), "plain"), (zip::Plan::zip64(entries.clone()), "zip64")] {
        let mut out = Vec::new();
        let sent = plan.write(&mut out, &AtomicBool::new(false)).unwrap();
        assert_eq!((sent, out.len() as u64), (plan.len(), plan.len()), "{name}: the length promised before the first byte");
        let read = unzip(&out);
        assert_eq!(read.len(), all.len());
        for ((got_name, data), item) in read.iter().zip(&all) {
            assert_eq!(got_name, &item.photo.name);
            assert_eq!(data, &std::fs::read(&item.photo.path).unwrap(), "{got_name}: stored as it is");
        }
        checked_by_others(&out, name);
    }
    // Its time is when the photo was taken (2026-11-21 14:03:12, in two-second steps).
    let at = NaiveDateTime::parse_from_str("2026-11-21 14:03:12", "%Y-%m-%d %H:%M:%S").unwrap();
    assert_eq!(zip::dos_time(at), ((14 << 11) | (3 << 5) | 6, ((2026 - 1980) << 9) | (11 << 5) | 21));
    // A photo changed after its length was promised: it stops, never sends a wrong archive.
    let plan = zip::Plan::new(entries.clone());
    std::fs::write(dir.join("IMG_20261120_091500.jpg"), b"shorter").unwrap();
    assert!(plan.write(&mut Vec::new(), &AtomicBool::new(false)).is_err());
    // Sharing stopped mid-way: it ends.
    assert!(zip::Plan::new(entries).write(&mut Vec::new(), &AtomicBool::new(true)).is_err());
    std::fs::remove_dir_all(&dir).ok();
}

/// One request over a real connection: the status line, the headers, the body.
fn over_the_network(port: u16, request: &str) -> (String, String, Vec<u8>) {
    let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    s.write_all(request.as_bytes()).unwrap();
    let mut all = Vec::new();
    s.read_to_end(&mut all).ok();
    let split = all.windows(4).position(|w| w == b"\r\n\r\n").map_or(all.len(), |p| p + 4);
    let head = String::from_utf8_lossy(&all[..split]).into_owned();
    let status = head.lines().next().unwrap_or("").to_owned();
    (status, head, all[split.min(all.len())..].to_vec())
}

fn serving(dir: &Path, open_for: Duration) -> (Gallery, u16) {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    listener.set_nonblocking(true).unwrap();
    let port = listener.local_addr().unwrap().port();
    (Gallery::serve_on(listener, "127.0.0.1", port, dir.to_path_buf(), look(), open_for).unwrap(), port)
}

#[test]
fn a_phone_on_the_network_gets_the_album_and_esc_closes_it_at_once() {
    let dir = album("loopback");
    let (gallery, port) = serving(&dir, serve::OPEN_FOR);
    assert_eq!(gallery.phones(), 0);
    let link = gallery.link().to_owned();
    let path = link.split_once("127.0.0.1").unwrap().1.split_once('/').map(|(_, p)| format!("/{p}")).unwrap();
    // The QR code's link: a cookie, and on to the page.
    let (status, head, _) = over_the_network(port, &format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n"));
    assert!(status.contains("303"), "{status}");
    let cookie = head.lines().find_map(|l| l.strip_prefix("Set-Cookie: ")).unwrap().split(';').next().unwrap().to_owned();
    assert_eq!(gallery.phones(), 1);
    let ask = |target: &str, extra: &str| over_the_network(port, &format!("GET {target} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nCookie: {cookie}\r\n{extra}\r\n"));
    let (status, _, page) = ask("/", "");
    let page = String::from_utf8(page).unwrap();
    assert!(status.contains("200") && page.contains("IMG_20261121_140312.jpg"));
    assert!(!page.contains(&link[link.len() - 26..]), "the token is not on the page");
    // A photo whole, and a range of it as a phone's video player asks.
    let photo = std::fs::read(dir.join("IMG_20261121_140312.jpg")).unwrap();
    let (status, head, body) = ask("/p/IMG_20261121_140312.jpg", "");
    assert!(status.contains("200") && head.contains("Content-Type: image/jpeg") && head.contains("Accept-Ranges: bytes"));
    assert_eq!(body, photo);
    let (status, _, body) = ask("/p/IMG_20261121_140312.jpg", "Range: bytes=10-19\r\n");
    assert!(status.contains("206"));
    assert_eq!(body, &photo[10..20]);
    // HEAD: the head alone.
    let (status, head, body) = over_the_network(port, &format!("HEAD /p/IMG_20261121_140312.jpg HTTP/1.1\r\nHost: x.local\r\nCookie: {cookie}\r\n\r\n"));
    assert!(status.contains("200") && head.contains(&format!("Content-Length: {}", photo.len())) && body.is_empty());
    // Everything as one ZIP, of the length it said.
    let (status, head, body) = ask("/zip/all", "");
    assert!(status.contains("200") && head.contains("application/zip") && head.contains(&format!("Content-Length: {}", body.len())));
    assert_eq!(unzip(&body).len(), 3);
    // Some of them: chosen, then fetched.
    let pick = "IMG_20261120_091500.jpg\nnot-in-the-album.jpg\n";
    let (status, _, _) = over_the_network(port, &format!("POST /select HTTP/1.1\r\nHost: 127.0.0.1\r\nCookie: {cookie}\r\nContent-Type: text/plain\r\nContent-Length: {}\r\n\r\n{pick}", pick.len()));
    assert!(status.contains("204"), "{status}");
    let (_, _, body) = ask("/zip/selected", "");
    let got = unzip(&body);
    assert_eq!(got.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(), ["IMG_20261120_091500.jpg"]);
    // Esc: a phone's connection open now is closed at once, and nothing answers after.
    let mut waiting = TcpStream::connect(("127.0.0.1", port)).unwrap();
    std::thread::sleep(Duration::from_millis(150));
    gallery.stop(Stopped::ByYou);
    waiting.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
    let mut buf = [0u8; 16];
    let started = std::time::Instant::now();
    let n = waiting.read(&mut buf).unwrap_or(0);
    assert!(n == 0 && started.elapsed() < Duration::from_secs(2), "closed, not left waiting");
    assert_eq!(gallery.stopped(), Some(Stopped::ByYou));
    std::thread::sleep(Duration::from_millis(200));
    assert!(TcpStream::connect_timeout(&format!("127.0.0.1:{port}").parse().unwrap(), Duration::from_millis(500)).is_err(), "no longer listening");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn it_stops_by_itself_once_its_time_is_up_and_use_renews_it() {
    let dir = album("time");
    let (gallery, port) = serving(&dir, Duration::from_millis(1500));
    assert!(gallery.left() > Duration::from_millis(1200));
    std::thread::sleep(Duration::from_millis(600));
    // Used: the time starts again.
    let shared = gallery.shared();
    let cookie = let_in(shared);
    let _ = get(shared, &cookie, "/list.json");
    assert!(gallery.left() > Duration::from_millis(1300), "renewed by use: {:?}", gallery.left());
    // `/ping` says the page is there, but is not use.
    std::thread::sleep(Duration::from_millis(500));
    let _ = get(shared, &cookie, "/ping");
    assert!(gallery.left() < Duration::from_millis(1050), "{:?}", gallery.left());
    std::thread::sleep(Duration::from_millis(1600));
    assert_eq!(gallery.stopped(), Some(Stopped::TimeUp));
    assert!(TcpStream::connect_timeout(&format!("127.0.0.1:{port}").parse().unwrap(), Duration::from_millis(500)).is_err());
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn the_page_is_one_file_in_the_handhelds_colours() {
    let dir = album("page");
    let html = page::page(&look(), &items(&dir));
    // Nothing from elsewhere: every address is the handheld's own.
    for outside in ["src=\"http", "href=\"http", "src=\"//", "url(http", "url(//", "@import", "cdn."] {
        assert!(!html.contains(outside), "{outside}");
    }
    // The palette of the accent in use, dark and light, from the shell's tokens.
    for tone in [bezel::Tone::Dark, bezel::Tone::Light] {
        let t = bezel::Tokens::of(bezel::Accent::Rose, tone);
        for c in [t.ground, t.surface1, t.ink, t.signal] {
            assert!(html.contains(&format!("#{:02x}{:02x}{:02x}", c.0, c.1, c.2)), "{tone:?}");
        }
    }
    assert!(html.contains("prefers-color-scheme:light") && html.contains("prefers-reduced-motion"));
    // Detent's springs, by role.
    assert!(html.contains("\"page\":[") && html.contains("linear("));
    // Share only where the browser has it.
    assert!(html.contains("navigator.canShare"));
    assert!(!html.contains("{{"), "every blank filled");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn the_sheets_keys_fit_and_esc_stops_sharing() {
    let mut g = crate::gfx::Gfx::new(bezel::Tokens::dark());
    let dir = album("sheet");
    let (gallery, port) = serving(&dir, serve::OPEN_FOR);
    let mut screen = GalleryScreen::around(gallery, dir.clone(), look());
    crate::keysheet::fits(&mut g, &screen.key_sheet()).unwrap();
    for showing in [super::sheet::Showing::NoNetwork, super::sheet::Showing::Stopped(Stopped::TimeUp), super::sheet::Showing::Trouble(Trouble::NoPort)] {
        crate::keysheet::fits(&mut g, &GalleryScreen::showing(dir.clone(), look(), showing).key_sheet()).unwrap();
    }
    assert!(screen.keeps_awake() && !screen.keep_in_recents());
    let mut robot = crate::robot::Robot::none();
    let mut settings = crate::shell::Settings::default();
    let mut say = None;
    let nav = screen.key(crate::platform::Key::Back, &mut crate::shell::Cx { robot: &mut robot, settings: &mut settings, say: &mut say, at_rest: true });
    assert!(matches!(nav, crate::shell::Nav::Pop));
    std::thread::sleep(Duration::from_millis(200));
    assert!(TcpStream::connect_timeout(&format!("127.0.0.1:{port}").parse().unwrap(), Duration::from_millis(500)).is_err(), "Esc: no longer listening");
    std::fs::remove_dir_all(&dir).ok();
}
