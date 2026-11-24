//! Tailscale, looked at and never changed: whether it is installed and
//! signed in, this computer's name on the tailnet, and whether Funnel
//! publishes the bridge's port, which gives the address the handheld uses.
//! Where something is missing, the exact steps to take, for the person to
//! take (`steps`). The bridge runs only `tailscale status --json` and
//! `tailscale funnel status --json`.

use std::path::PathBuf;
use std::process::Command;

use serde_json::Value;

/// The ports Funnel can publish on.
pub const FUNNEL_PORTS: [u16; 3] = [443, 8443, 10000];

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Tailscale {
    pub installed: bool,
    /// Signed in and connected.
    pub running: bool,
    /// This computer's name on the tailnet (`laptop.tail1234.ts.net`).
    pub dns_name: Option<String>,
    /// The address Funnel publishes the bridge at, if it does.
    pub address: Option<String>,
    /// Funnel ports this computer already uses for something else.
    pub taken: Vec<u16>,
}

/// Where `tailscale` is.
pub fn exe() -> Option<PathBuf> {
    let mut places: Vec<PathBuf> = Vec::new();
    for var in ["ProgramFiles", "ProgramW6432"] {
        if let Some(p) = std::env::var_os(var) {
            places.push(PathBuf::from(p).join("Tailscale").join("tailscale.exe"));
        }
    }
    let name = if cfg!(windows) { "tailscale.exe" } else { "tailscale" };
    places.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()).map(|d| d.join(name)));
    places.into_iter().find(|p| p.is_file())
}

fn run_json(exe: &PathBuf, args: &[&str]) -> Option<Value> {
    let mut cmd = Command::new(exe);
    cmd.args(args);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let out = cmd.output().ok()?;
    serde_json::from_slice(&out.stdout).ok()
}

/// Tailscale as it is now, for the bridge on `port`.
pub fn look(port: u16) -> Tailscale {
    let Some(exe) = exe() else { return Tailscale::default() };
    let status = run_json(&exe, &["status", "--json"]);
    let funnel = run_json(&exe, &["funnel", "status", "--json"]);
    read(status.as_ref(), funnel.as_ref(), port)
}

/// What the two answers say, for the bridge on `port`.
pub fn read(status: Option<&Value>, funnel: Option<&Value>, port: u16) -> Tailscale {
    let mut t = Tailscale { installed: true, ..Tailscale::default() };
    let Some(status) = status else { return t };
    t.running = status["BackendState"] == "Running";
    t.dns_name = status["Self"]["DNSName"].as_str().map(|s| s.trim_end_matches('.').to_owned()).filter(|s| !s.is_empty());
    let Some(funnel) = funnel else { return t };
    let ours = |proxy: &str| {
        let p = proxy.trim_end_matches('/');
        [format!("http://127.0.0.1:{port}"), format!("http://localhost:{port}"), format!("127.0.0.1:{port}"), format!("localhost:{port}")].iter().any(|x| x == p)
    };
    for (hostport, web) in funnel["Web"].as_object().into_iter().flatten() {
        let Some((host, p)) = hostport.rsplit_once(':') else { continue };
        let Ok(p) = p.parse::<u16>() else { continue };
        let public = funnel["AllowFunnel"][hostport.as_str()] == true;
        let handlers = web["Handlers"].as_object();
        let mine = handlers.into_iter().flatten().any(|(path, h)| path == "/" && h["Proxy"].as_str().is_some_and(ours));
        if mine && public {
            t.address = Some(if p == 443 { format!("https://{host}") } else { format!("https://{host}:{p}") });
        } else if handlers.is_some_and(|h| !h.is_empty()) {
            t.taken.push(p);
        }
    }
    // A port given to raw TCP forwarding is taken too.
    for (p, tcp) in funnel["TCP"].as_object().into_iter().flatten() {
        if let Ok(p) = p.parse::<u16>() {
            if tcp["HTTPS"] != true && !t.taken.contains(&p) {
                t.taken.push(p);
            }
        }
    }
    t.taken.sort_unstable();
    t
}

/// The steps left before the handheld can reach the bridge, each a line;
/// none when it can.
pub fn steps(t: &Tailscale, port: u16) -> Vec<String> {
    let mut s = Vec::new();
    if !t.installed {
        s.push("Install Tailscale for Windows from https://tailscale.com/download/windows and sign in.".to_owned());
    } else if !t.running {
        s.push("Open Tailscale from the tray and sign in, so it says Connected.".to_owned());
    }
    if t.address.is_none() {
        let free = FUNNEL_PORTS.iter().copied().find(|p| !t.taken.contains(p)).unwrap_or(10000);
        let https = if free == 443 { String::new() } else { format!(" --https={free}") };
        s.push("In the Tailscale admin console (login.tailscale.com/admin/dns), turn on MagicDNS and HTTPS certificates.".to_owned());
        s.push(format!("In a terminal, run:  tailscale funnel --bg{https} {port}"));
        s.push("The first time, it prints a link to allow Funnel for this computer: open it and allow it, then run the command again.".to_owned());
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn funnel_on_the_bridge_gives_its_address() {
        let status = json!({"BackendState": "Running", "Self": {"DNSName": "laptop.tail1234.ts.net."}});
        let funnel = json!({
            "TCP": {"443": {"HTTPS": true}, "10000": {"HTTPS": true}},
            "Web": {
                "laptop.tail1234.ts.net:443": {"Handlers": {"/": {"Proxy": "http://127.0.0.1:8080"}}},
                "laptop.tail1234.ts.net:10000": {"Handlers": {"/": {"Proxy": "http://127.0.0.1:47318"}}}
            },
            "AllowFunnel": {"laptop.tail1234.ts.net:443": true, "laptop.tail1234.ts.net:10000": true}
        });
        let t = read(Some(&status), Some(&funnel), 47318);
        assert!(t.running);
        assert_eq!(t.dns_name.as_deref(), Some("laptop.tail1234.ts.net"));
        assert_eq!(t.address.as_deref(), Some("https://laptop.tail1234.ts.net:10000"));
        assert_eq!(t.taken, [443]);
        assert!(steps(&t, 47318).is_empty());
    }

    #[test]
    fn without_funnel_the_steps_name_a_free_port() {
        let status = json!({"BackendState": "Running", "Self": {"DNSName": "laptop.tail1234.ts.net."}});
        let funnel = json!({"Web": {"laptop.tail1234.ts.net:443": {"Handlers": {"/": {"Proxy": "http://127.0.0.1:8080"}}}, "laptop.tail1234.ts.net:8443": {"Handlers": {"/": {"Proxy": "http://127.0.0.1:8080"}}}}, "AllowFunnel": {}});
        let t = read(Some(&status), Some(&funnel), 47318);
        assert_eq!(t.address, None);
        let s = steps(&t, 47318);
        assert!(s.iter().any(|l| l.contains("tailscale funnel --bg --https=10000 47318")), "{s:?}");
        // Served only inside the tailnet is not published.
        let inside = json!({"Web": {"laptop.tail1234.ts.net:443": {"Handlers": {"/": {"Proxy": "http://127.0.0.1:47318"}}}}, "AllowFunnel": {}});
        assert_eq!(read(Some(&status), Some(&inside), 47318).address, None);
        let none = Tailscale::default();
        assert!(steps(&none, 47318)[0].starts_with("Install Tailscale"));
        assert!(steps(&none, 47318).iter().any(|l| l.ends_with("tailscale funnel --bg 47318")));
    }
}
