//! The handheld's files as a computer sees them: the music library, match
//! logs, photos, screenshots, notes and strategy boards' pictures (listed
//! and downloaded), songs
//! added to the library, and /data backed up and restored as a tar stream.

use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use catalyst_companion_proto::tar;
use serde_json::{json, Value};

use crate::store::Paths;

/// The places a computer may look in, and where each is under /data.
pub const AREAS: [(&str, &str); 6] = [("music", "music/library"), ("logs", "logs"), ("photos", "photos"), ("screenshots", "screenshots"), ("notes", "notes"), ("strategy", "strategy")];

/// Songs the library takes from a computer.
pub const SONGS: [&str; 7] = ["mp3", "m4a", "flac", "opus", "ogg", "aac", "wav"];

/// Where songs from a computer go in the library (tags still say where
/// each belongs in Music).
pub const IMPORTED: &str = "From computer";

/// Never in a backup, never written by a restore: what is made again or
/// fetched again (caches, downloaded updates), the pairing itself, and
/// YouTube Music's sign-in, which is never kept on a computer (signing in
/// again is a click in the app).
const LEFT_OUT: [&str; 6] = ["companion", "update", "music/cache", "music/cookies.txt", "lost+found", "tailscale/tailscaled.log.conf"];

pub fn left_out(rel: &str) -> bool {
    LEFT_OUT.iter().any(|p| rel == *p || rel.starts_with(&format!("{p}/"))) || rel.split('/').any(|part| part == "cache" || part == ".cache" || part.ends_with(".catalyst-saving") || part.ends_with(".part"))
}

fn area_root(paths: &Paths, area: &str) -> Option<PathBuf> {
    AREAS.iter().find(|(name, _)| *name == area).map(|(_, rel)| paths.data.join(rel))
}

/// A file or folder in an area, by its place in it; never outside it
/// (no `..`, and no link leading out).
pub fn resolve(paths: &Paths, area: &str, rel: &str) -> Result<PathBuf, String> {
    let root = area_root(paths, area).ok_or("no such place")?;
    let rel = rel.trim_matches('/');
    let full = if rel.is_empty() {
        root.clone()
    } else {
        let parts = tar::safe_path(rel).ok_or("not a place in it")?;
        parts.iter().fold(root.clone(), |p, part| p.join(part))
    };
    let (Ok(real), Ok(real_root)) = (std::fs::canonicalize(&full), std::fs::canonicalize(&root)) else {
        return Err("not there".into());
    };
    if !real.starts_with(&real_root) {
        return Err("not a place in it".into());
    }
    Ok(full)
}

fn mtime(meta: &std::fs::Metadata) -> u64 {
    meta.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs())
}

/// A folder's entries: folders first, then files, by name; hidden ones (the
/// library's index) left out.
pub fn list(paths: &Paths, area: &str, rel: &str) -> Result<Value, String> {
    let root = area_root(paths, area).ok_or("no such place")?;
    std::fs::create_dir_all(&root).ok();
    let dir = resolve(paths, area, rel)?;
    let mut entries: Vec<(bool, String, u64, u64)> = std::fs::read_dir(&dir)
        .map_err(|_| "not a folder")?
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let meta = e.metadata().ok()?;
            (!name.starts_with('.') && (meta.is_dir() || meta.is_file())).then(|| (meta.is_dir(), name, meta.len(), mtime(&meta)))
        })
        .collect();
    entries.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.to_lowercase().cmp(&b.1.to_lowercase())));
    let entries: Vec<Value> = entries.into_iter().map(|(dir, name, size, modified)| json!({"name": name, "dir": dir, "size": if dir { 0 } else { size }, "modified": modified})).collect();
    Ok(json!({"area": area, "path": rel.trim_matches('/'), "entries": entries}))
}

/// What a file sent to the library may be called: its own name, without
/// folders or anything a file name may not have, and a song's extension.
pub fn song_name(name: &str) -> Result<String, String> {
    let name = name.rsplit(['/', '\\']).next().unwrap_or_default().trim();
    let clean: String = name.chars().filter(|c| !c.is_control() && !"<>:\"|?*".contains(*c)).collect();
    let clean = clean.trim_start_matches('.').trim().to_owned();
    let ext = clean.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    if clean.is_empty() || clean.len() > 200 {
        return Err("that name will not do for a song".into());
    }
    if !SONGS.contains(&ext.as_str()) {
        return Err(format!("Music plays {}; that is not one", SONGS.join(", ")));
    }
    Ok(clean)
}

/// The space left on /data, in bytes.
pub fn free_space(path: &Path) -> Option<u64> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        let c = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
        let mut s: libc::statvfs = unsafe { std::mem::zeroed() };
        // SAFETY: `c` is a NUL-terminated path and `s` is ours to fill.
        if unsafe { libc::statvfs(c.as_ptr(), &mut s) } != 0 {
            return None;
        }
        Some(s.f_bavail as u64 * s.f_frsize as u64)
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        None
    }
}

/// A song from a computer, `len` bytes of `body`, into the library:
/// written aside, then put in place (a name taken gets a number).
pub fn add_song(paths: &Paths, name: &str, len: u64, body: &mut dyn Read) -> Result<String, String> {
    let name = song_name(name)?;
    let dir = paths.data.join("music/library").join(IMPORTED);
    std::fs::create_dir_all(&dir).map_err(|e| format!("could not make the folder: {e}"))?;
    if let Some(free) = free_space(&dir) {
        if len + (64 << 20) > free {
            return Err("there is not room for it on the handheld".into());
        }
    }
    let (stem, ext) = name.rsplit_once('.').unwrap_or((&name, ""));
    let mut target = dir.join(&name);
    let mut n = 2;
    while target.exists() {
        target = dir.join(format!("{stem} ({n}).{ext}"));
        n += 1;
    }
    let part = dir.join(format!(".{}.part", target.file_name().unwrap_or_default().to_string_lossy()));
    let result = (|| -> io::Result<()> {
        let mut f = std::fs::File::create(&part)?;
        let copied = io::copy(&mut body.take(len), &mut f)?;
        if copied != len {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "the song stopped coming"));
        }
        f.sync_all()?;
        std::fs::rename(&part, &target)
    })();
    if let Err(e) = result {
        std::fs::remove_file(&part).ok();
        return Err(format!("the song did not arrive whole: {e}"));
    }
    Ok(format!("{IMPORTED}/{}", target.file_name().unwrap_or_default().to_string_lossy()))
}

/// What a backup holds: every file and folder under /data but those left
/// out, and no links.
pub fn backup_list(data: &Path) -> Vec<tar::Entry> {
    let mut out = Vec::new();
    let mut stack = vec![String::new()];
    while let Some(rel) = stack.pop() {
        let dir = if rel.is_empty() { data.to_path_buf() } else { data.join(&rel) };
        let mut entries: Vec<_> = std::fs::read_dir(&dir).into_iter().flatten().flatten().collect();
        entries.sort_by_key(|e| std::cmp::Reverse(e.file_name()));
        for e in entries {
            let name = e.file_name().to_string_lossy().into_owned();
            let path = if rel.is_empty() { name } else { format!("{rel}/{name}") };
            let Ok(meta) = std::fs::symlink_metadata(e.path()) else { continue };
            if left_out(&path) || meta.file_type().is_symlink() {
                continue;
            }
            #[cfg(unix)]
            let mode = std::os::unix::fs::PermissionsExt::mode(&meta.permissions()) & 0o7777;
            #[cfg(not(unix))]
            let mode = if meta.is_dir() { 0o755 } else { 0o644 };
            if meta.is_dir() {
                out.push(tar::Entry { path: path.clone(), mode, size: 0, mtime: mtime(&meta), dir: true });
                stack.push(path);
            } else if meta.is_file() {
                out.push(tar::Entry { path, mode, size: meta.len(), mtime: mtime(&meta), dir: false });
            }
        }
    }
    out
}

/// The stream's length for `list`.
pub fn backup_len(list: &[tar::Entry]) -> u64 {
    list.iter().map(|e| tar::entry_len(&e.path, e.size, e.dir)).sum::<u64>() + tar::END.len() as u64
}

/// The backup written: each file as long as it was listed (one that shrank
/// since is made up with zeros, one that grew is cut), so the stream is
/// exactly `backup_len` long.
pub fn write_backup(data: &Path, list: &[tar::Entry], out: &mut dyn Write) -> io::Result<()> {
    for e in list {
        if e.dir {
            tar::write_entry(out, e, &mut io::empty())?;
            continue;
        }
        let file: Box<dyn Read> = match std::fs::File::open(data.join(&e.path)) {
            Ok(f) => Box::new(f.take(e.size)),
            Err(_) => Box::new(io::empty()),
        };
        tar::write_entry(out, e, &mut file.chain(io::repeat(0)).take(e.size))?;
    }
    out.write_all(&tar::END)
}

/// A backup put back under /data: files written aside and moved into
/// place, with their permissions. Paths that would leave /data, and what
/// backups leave out, are passed over. The number of files written.
pub fn restore(data: &Path, body: &mut dyn Read) -> Result<usize, String> {
    let mut reader = tar::Reader::new(body);
    let mut files = 0;
    while let Some(entry) = reader.next().map_err(|e| format!("the backup did not read: {e}"))? {
        let Some(parts) = tar::safe_path(&entry.path) else { continue };
        if left_out(&entry.path) {
            continue;
        }
        let target = parts.iter().fold(data.to_path_buf(), |p, part| p.join(part));
        // Never through a link someone left in /data.
        if parts.iter().scan(data.to_path_buf(), |p, part| { *p = p.join(part); Some(p.clone()) }).any(|p| std::fs::symlink_metadata(&p).is_ok_and(|m| m.file_type().is_symlink())) {
            continue;
        }
        if entry.dir {
            std::fs::create_dir_all(&target).map_err(|e| format!("could not make {}: {e}", entry.path))?;
            continue;
        }
        if let Some(dir) = target.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("could not make the folder for {}: {e}", entry.path))?;
        }
        let mut name = target.file_name().unwrap_or_default().to_owned();
        name.push(".catalyst-saving");
        let temp = target.with_file_name(name);
        let written = (|| -> io::Result<()> {
            let mut f = std::fs::File::create(&temp)?;
            io::copy(&mut reader.body(), &mut f)?;
            f.sync_all()?;
            #[cfg(unix)]
            std::fs::set_permissions(&temp, std::os::unix::fs::PermissionsExt::from_mode(entry.mode & 0o777))?;
            std::fs::rename(&temp, &target)
        })();
        if let Err(e) = written {
            std::fs::remove_file(&temp).ok();
            return Err(format!("could not put back {}: {e}", entry.path));
        }
        files += 1;
    }
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> Paths {
        let root = std::env::temp_dir().join(format!("companiond-files-{name}-{}", std::process::id()));
        std::fs::remove_dir_all(&root).ok();
        let paths = Paths { data: root.join("data"), run: root.join("run") };
        std::fs::create_dir_all(&paths.data).unwrap();
        std::fs::create_dir_all(&paths.run).unwrap();
        paths
    }

    #[test]
    fn an_area_lists_folders_first_and_never_reaches_outside_itself() {
        let p = scratch("list");
        let lib = p.data.join("music/library");
        std::fs::create_dir_all(lib.join("Artist/Album")).unwrap();
        std::fs::write(lib.join("b.mp3"), b"12345").unwrap();
        std::fs::write(lib.join(".index"), b"{}").unwrap();
        std::fs::write(p.data.join("keys"), b"secret").unwrap();
        let v = list(&p, "music", "").unwrap();
        let names: Vec<&str> = v["entries"].as_array().unwrap().iter().map(|e| e["name"].as_str().unwrap()).collect();
        assert_eq!(names, ["Artist", "b.mp3"], "folders first, the index hidden");
        assert_eq!(v["entries"][1]["size"], 5);
        assert!(resolve(&p, "music", "../../keys").is_err());
        assert!(resolve(&p, "music", "Artist/../../../keys").is_err());
        assert!(resolve(&p, "nowhere", "").is_err());
        assert!(resolve(&p, "music", "b.mp3").is_ok());
    }

    #[test]
    fn a_song_arrives_whole_beside_the_others_and_only_a_song() {
        let p = scratch("songs");
        let rel = add_song(&p, "C:\\Music\\My Song.mp3", 4, &mut &b"abcdefgh"[..]).unwrap();
        assert_eq!(rel, "From computer/My Song.mp3");
        assert_eq!(std::fs::read(p.data.join("music/library").join(&rel)).unwrap(), b"abcd");
        let again = add_song(&p, "My Song.mp3", 2, &mut &b"xy"[..]).unwrap();
        assert_eq!(again, "From computer/My Song (2).mp3", "a name taken gets a number");
        assert!(add_song(&p, "notes.txt", 1, &mut &b"x"[..]).is_err());
        assert!(add_song(&p, "short.flac", 10, &mut &b"abc"[..]).is_err(), "one that stops coming is not kept");
        let left: Vec<_> = std::fs::read_dir(p.data.join("music/library").join(IMPORTED)).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        assert_eq!(left.len(), 2, "nothing half-written is left: {left:?}");
    }

    #[test]
    fn a_backup_leaves_out_caches_and_the_pairing_and_restores_as_it_was() {
        let p = scratch("backup");
        let d = &p.data;
        for (rel, body) in [("catalyst/settings", "accent=rose\n"), ("keys", "tba=x\n"), ("music/library/A/1.mp3", "song"), ("music/cache/x", "cached"), ("music/cookies.txt", "# Netscape"), ("companion/computers.json", "[]"), ("update/system.img", "big"), ("notes/a.txt", "hello")] {
            std::fs::create_dir_all(d.join(rel).parent().unwrap()).unwrap();
            std::fs::write(d.join(rel), body).unwrap();
        }
        let list = backup_list(d);
        let names: Vec<&str> = list.iter().filter(|e| !e.dir).map(|e| e.path.as_str()).collect();
        assert!(names.contains(&"catalyst/settings") && names.contains(&"keys") && names.contains(&"music/library/A/1.mp3") && names.contains(&"notes/a.txt"));
        assert!(!names.iter().any(|n| n.starts_with("music/cache") || n.starts_with("companion") || n.starts_with("update") || n.ends_with("cookies.txt")), "{names:?}");
        let mut stream = Vec::new();
        write_backup(d, &list, &mut stream).unwrap();
        assert_eq!(stream.len() as u64, backup_len(&list));

        let q = scratch("restore");
        std::fs::create_dir_all(q.data.join("companion")).unwrap();
        std::fs::write(q.data.join("companion/computers.json"), "mine").unwrap();
        let n = restore(&q.data, &mut &stream[..]).unwrap();
        assert_eq!(n, names.len());
        assert_eq!(std::fs::read_to_string(q.data.join("catalyst/settings")).unwrap(), "accent=rose\n");
        assert_eq!(std::fs::read_to_string(q.data.join("companion/computers.json")).unwrap(), "mine", "this handheld's pairing stays its own");
    }

    #[test]
    fn a_restore_never_writes_outside_data() {
        let p = scratch("evil");
        let mut stream = Vec::new();
        for path in ["../escaped", "ok.txt", "companion/computers.json"] {
            let e = tar::Entry { path: path.into(), mode: 0o644, size: 1, mtime: 0, dir: false };
            tar::write_entry(&mut stream, &e, &mut &b"x"[..]).unwrap();
        }
        stream.extend_from_slice(&tar::END);
        assert_eq!(restore(&p.data, &mut &stream[..]).unwrap(), 1);
        assert!(!p.data.parent().unwrap().join("escaped").exists());
        assert!(p.data.join("ok.txt").exists() && !p.data.join("companion").exists());
    }
}
