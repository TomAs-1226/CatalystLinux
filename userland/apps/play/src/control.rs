//! What the shell tells the player while it plays, a line at a time on
//! standard input (fields separated by tabs, so a path may hold spaces):
//!
//! ```text
//! next<TAB>PATH[<TAB>size=BYTES][<TAB>album=KEY][<TAB>seconds=S]
//!                         the song after this one: opened ahead, and run
//!                         on into without a gap (or crossfaded, as
//!                         Settings › Sound says, unless KEY is this song's
//!                         album too); a later `next` replaces it
//! next                    no song after this one after all
//! album<TAB>KEY           the album of the song playing (the first one's)
//! fade<TAB>SECONDS[<TAB>LEVEL]
//!                         the whole sound faded to LEVEL (0 silent, the
//!                         default; 1 as it was) over SECONDS: a sleep
//!                         timer's fade, or its undoing; `faded` is said
//!                         when it gets there
//! ```
//!
//! Standard input closed (or never opened: the shell's older players, the
//! tests), nothing changes: the song plays to its end.

use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver};

#[derive(Clone, Debug, PartialEq)]
pub struct Next {
    pub path: PathBuf,
    pub size: Option<u64>,
    pub album: String,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    Next(Option<Next>),
    Album(String),
    Fade { seconds: f32, to: f32 },
}

pub fn parse(line: &str) -> Option<Command> {
    let line = line.trim_end_matches(['\r', '\n']);
    let mut f = line.split('\t');
    match f.next()?.trim() {
        "next" => {
            let Some(path) = f.next().filter(|p| !p.trim().is_empty()) else { return Some(Command::Next(None)) };
            let mut next = Next { path: PathBuf::from(path), size: None, album: String::new() };
            for kv in f {
                match kv.split_once('=') {
                    Some(("size", v)) => next.size = v.trim().parse().ok(),
                    Some(("album", v)) => next.album = v.to_owned(),
                    _ => {}
                }
            }
            Some(Command::Next(Some(next)))
        }
        "album" => Some(Command::Album(f.next().unwrap_or_default().to_owned())),
        "fade" => {
            let seconds = f.next()?.trim().parse::<f32>().ok().filter(|s| s.is_finite() && *s >= 0.0)?;
            let to = f.next().and_then(|v| v.trim().parse::<f32>().ok()).unwrap_or(0.0).clamp(0.0, 1.0);
            Some(Command::Fade { seconds, to })
        }
        _ => None,
    }
}

/// Standard input's commands, read by a thread of their own.
pub fn listen() -> Receiver<Command> {
    let (tx, rx) = channel();
    std::thread::Builder::new()
        .name("commands".into())
        .stack_size(64 * 1024)
        .spawn(move || {
            use std::io::BufRead;
            for line in std::io::stdin().lock().lines().map_while(Result::ok) {
                if let Some(c) = parse(&line) {
                    if tx.send(c).is_err() {
                        break;
                    }
                }
            }
        })
        .ok();
    rx
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shells_lines_are_read() {
        assert_eq!(
            parse("next\t/data/music/library/A B/01 Song.flac\talbum=a b|rumours\tsize=123\n"),
            Some(Command::Next(Some(Next { path: "/data/music/library/A B/01 Song.flac".into(), size: Some(123), album: "a b|rumours".into() })))
        );
        assert_eq!(parse("next"), Some(Command::Next(None)));
        assert_eq!(parse("next\t"), Some(Command::Next(None)));
        assert_eq!(parse("fade\t30"), Some(Command::Fade { seconds: 30.0, to: 0.0 }));
        assert_eq!(parse("fade\t2\t1"), Some(Command::Fade { seconds: 2.0, to: 1.0 }));
        assert_eq!(parse("fade\tsoon"), None);
        assert_eq!(parse("album\tx|y"), Some(Command::Album("x|y".into())));
        assert_eq!(parse("dance"), None);
    }
}
