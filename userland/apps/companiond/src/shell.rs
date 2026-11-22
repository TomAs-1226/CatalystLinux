//! The shell does what only it can: settings put in place while it runs
//! (the look, the team), and the sign-ins it checks and keeps (YouTube
//! Music's cookies, keys), with the very code its page for setting up from
//! a computer uses. It listens on a socket only root can open; a request
//! is a command line, then its body; the answer is `ok` and JSON, or
//! `error` and why.

use std::io::{Read, Write};
use std::path::PathBuf;

pub trait Shell: Send + Sync {
    /// `command` with `body`; the shell's answer, or why there was none.
    fn ask(&self, command: &str, body: &str) -> Result<String, String>;
}

/// The running shell, on its socket (or, for the simulator on a computer,
/// a port on this computer alone).
pub struct Socket {
    #[cfg_attr(not(unix), allow(dead_code))]
    pub path: PathBuf,
    pub tcp: Option<String>,
}

const GONE: &str = "the handheld's shell is not running";

/// `ok\n<json>` → the JSON; `error <why>` → why.
pub fn answer(reply: &str) -> Result<String, String> {
    if let Some(rest) = reply.strip_prefix("ok\n").or(reply.strip_prefix("ok")) {
        return Ok(rest.to_owned());
    }
    Err(reply.strip_prefix("error ").map_or_else(|| GONE.to_owned(), |why| why.trim().to_owned()))
}

/// Sends the request, closes this end for writing, and reads the answer.
fn exchange<S>(stream: S, request: &str, shut: impl FnOnce(&S)) -> Result<String, String>
where
    for<'a> &'a S: Read + Write,
{
    (&stream).write_all(request.as_bytes()).map_err(|_| GONE.to_owned())?;
    shut(&stream);
    let mut reply = String::new();
    (&stream).read_to_string(&mut reply).map_err(|_| GONE.to_owned())?;
    answer(&reply)
}

impl Shell for Socket {
    fn ask(&self, command: &str, body: &str) -> Result<String, String> {
        let timeout = Some(std::time::Duration::from_secs(20));
        let request = format!("{command}\n{body}");
        if let Some(addr) = &self.tcp {
            let s = std::net::TcpStream::connect(addr).map_err(|_| GONE.to_owned())?;
            s.set_read_timeout(timeout).ok();
            return exchange(s, &request, |s| drop(s.shutdown(std::net::Shutdown::Write)));
        }
        #[cfg(unix)]
        {
            let s = std::os::unix::net::UnixStream::connect(&self.path).map_err(|_| GONE.to_owned())?;
            s.set_read_timeout(timeout).ok();
            exchange(s, &request, |s| drop(s.shutdown(std::net::Shutdown::Write)))
        }
        #[cfg(not(unix))]
        Err(GONE.to_owned())
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_shells_answers_read_as_json_or_why() {
        assert_eq!(super::answer("ok\n{\"a\":1}"), Ok("{\"a\":1}".into()));
        assert_eq!(super::answer("error those cookies are not signed in\n"), Err("those cookies are not signed in".into()));
        assert!(super::answer("").is_err());
    }
}
