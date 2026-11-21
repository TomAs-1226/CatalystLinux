//! Share: something on the handheld as a QR code, for a phone to scan. This
//! Wi-Fi network (a phone joins it), typed text or a link, SSH to this
//! handheld, the team's page on The Blue Alliance.
//!
//! The Wi-Fi network's password is read from wpa_supplicant's own file;
//! one saved only as a derived key (64 hex digits) cannot be shared, and
//! says so. The code is drawn large, dark on white with its quiet margin.

use bezel::{Align, Color, Rect};

use crate::gfx::{Gfx, ListState, H, PAD, W};
use crate::platform::Key;
use crate::robot::Robot;
use crate::shell::{Cx, Nav, Screen};
use crate::tools::{Field, Typed};

const CONF: &str = "/data/wifi/wpa_supplicant.conf";

/// How a saved network is joined.
#[derive(Clone, Debug, PartialEq)]
pub enum Secret {
    Open,
    Password(String),
    /// Saved as the derived key only: nothing a phone could be given.
    KeyOnly,
}

/// A network block's `ssid=` value: quoted text, or hex as `set_network` writes it.
fn ssid_of(value: &str) -> Option<String> {
    let value = value.trim();
    if let Some(quoted) = value.strip_prefix('"').and_then(|v| v.strip_suffix('"')) {
        return Some(quoted.to_owned());
    }
    if value.len() % 2 != 0 || !value.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let bytes: Option<Vec<u8>> = (0..value.len()).step_by(2).map(|i| u8::from_str_radix(&value[i..i + 2], 16).ok()).collect();
    String::from_utf8(bytes?).ok()
}

/// How the network named `ssid` is joined, from wpa_supplicant's file.
pub fn saved_secret(conf: &str, ssid: &str) -> Option<Secret> {
    for block in conf.split("network={").skip(1) {
        let block = block.split('}').next().unwrap_or("");
        let field = |name: &str| block.lines().find_map(|l| l.trim().strip_prefix(name)?.strip_prefix('=').map(str::to_owned));
        if field("ssid").and_then(|s| ssid_of(&s)).as_deref() != Some(ssid) {
            continue;
        }
        return Some(match field("psk") {
            Some(p) if p.starts_with('"') && p.ends_with('"') && p.len() >= 2 => Secret::Password(p[1..p.len() - 1].to_owned()),
            Some(_) => Secret::KeyOnly,
            None if field("key_mgmt").as_deref() == Some("NONE") => Secret::Open,
            None => Secret::KeyOnly,
        });
    }
    None
}

/// The text a phone's camera reads as "join this network".
pub fn wifi_qr_text(ssid: &str, secret: &Secret) -> Option<String> {
    let escape = |s: &str| s.chars().fold(String::new(), |mut out, c| {
        if matches!(c, '\\' | ';' | ',' | ':' | '"') {
            out.push('\\');
        }
        out.push(c);
        out
    });
    match secret {
        Secret::Open => Some(format!("WIFI:T:nopass;S:{};;", escape(ssid))),
        Secret::Password(p) => Some(format!("WIFI:T:WPA;S:{};P:{};;", escape(ssid), escape(p))),
        Secret::KeyOnly => None,
    }
}

/// The Wi-Fi network joined now, and how it is joined.
fn current_wifi() -> Option<(String, Secret)> {
    if !crate::sys::on_handheld() {
        return Some(("Workshop".into(), Secret::Password("robots-in-the-pit".into())));
    }
    let out = std::process::Command::new("wpa_cli").args(["-p", "/run/wpa_supplicant", "-i", "wlan0", "status"]).output().ok()?;
    let (state, ssid, _) = crate::wifi::parse_status(&String::from_utf8_lossy(&out.stdout));
    let ssid = ssid.filter(|_| state == "COMPLETED")?;
    let conf = std::fs::read_to_string(CONF).unwrap_or_default();
    Some((ssid.clone(), saved_secret(&conf, &ssid).unwrap_or(Secret::KeyOnly)))
}

/// What can be shared.
#[derive(Clone, Debug, PartialEq)]
enum Item {
    Wifi(String, Secret),
    Text,
    Link,
    Ssh(String),
    Team(u32),
}

pub struct Share {
    items: Vec<Item>,
    list: ListState,
    typing: Option<(Field, bool)>,
}

impl Share {
    pub fn new(robot: &Robot) -> Self {
        let mut items = Vec::new();
        if let Some((ssid, secret)) = current_wifi() {
            items.push(Item::Wifi(ssid, secret));
        }
        items.push(Item::Text);
        items.push(Item::Link);
        let address = ["wlan0", "eth0"].iter().find_map(|i| crate::sys::address(i).filter(|a| a.contains('.')));
        if let Some(address) = address.or_else(|| (!crate::sys::on_handheld()).then(|| "192.168.0.42".to_owned())) {
            items.push(Item::Ssh(address));
        }
        if let Some(team) = crate::tools::team(robot) {
            items.push(Item::Team(team));
        }
        Share { items, list: ListState::default(), typing: None }
    }
}

impl Screen for Share {
    fn title(&self) -> &str {
        "Share"
    }

    fn wants_text(&self) -> bool {
        self.typing.is_some()
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        if let Some((field, link)) = &mut self.typing {
            let link = *link;
            match field.key(key, |c| !c.is_control()) {
                Typed::Done(text) if text.is_empty() => self.typing = None,
                Typed::Done(text) => {
                    self.typing = None;
                    let text = if link && !text.contains("://") { format!("https://{text}") } else { text };
                    let title = if link { "Link" } else { "Text" };
                    return match Code::new(title, &text, &text) {
                        Some(code) => Nav::Push(Box::new(code)),
                        None => {
                            *cx.say = Some("too long for a code".into());
                            Nav::Stay
                        }
                    };
                }
                Typed::Cancelled => self.typing = None,
                Typed::More => {}
            }
            return Nav::Stay;
        }
        match key {
            Key::Back => return Nav::Pop,
            Key::Enter | Key::Right => {
                let code = match self.items.get(self.list.focus).cloned() {
                    Some(Item::Wifi(ssid, secret)) => match wifi_qr_text(&ssid, &secret) {
                        Some(text) => Code::new("Wi-Fi", &text, &format!("joins {ssid}{}", if secret == Secret::Open { " (open)" } else { "" })),
                        None => {
                            *cx.say = Some("saved as a key only: it cannot be shared".into());
                            None
                        }
                    },
                    Some(Item::Text) => {
                        self.typing = Some((Field::new("text to share", "", 300), false));
                        None
                    }
                    Some(Item::Link) => {
                        self.typing = Some((Field::new("a link to share", "", 300), true));
                        None
                    }
                    Some(Item::Ssh(address)) => Code::new("SSH", &format!("ssh://root@{address}"), &format!("ssh root@{address}")),
                    Some(Item::Team(team)) => {
                        let url = format!("https://www.thebluealliance.com/team/{team}");
                        Code::new("The Blue Alliance", &url, &format!("team {team} on The Blue Alliance"))
                    }
                    None => None,
                };
                if let Some(code) = code {
                    return Nav::Push(Box::new(code));
                }
            }
            other => crate::screens::list_key(&mut self.list, other, self.items.len()),
        }
        Nav::Stay
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        if let Some((field, _)) = &self.typing {
            return field.draw(g, "enter: show it · esc: cancel");
        }
        let (top, row_h, bottom) = (g.top(), g.row_h(), H - PAD - g.hint_h());
        let items = self.items.clone();
        for (i, y) in self.list.layout(g, items.len(), top, row_h, 3.0, bottom) {
            let r = Rect::new(PAD, y, W - 2.0 * PAD - 5.0, row_h);
            let (name, value) = match &items[i] {
                Item::Wifi(ssid, Secret::KeyOnly) => ("This Wi-Fi".to_owned(), format!("{ssid} · key only")),
                Item::Wifi(ssid, _) => ("This Wi-Fi".to_owned(), ssid.clone()),
                Item::Text => ("Text".to_owned(), "type".to_owned()),
                Item::Link => ("Link".to_owned(), "type".to_owned()),
                Item::Ssh(address) => ("SSH to here".to_owned(), address.clone()),
                Item::Team(team) => ("Team page".to_owned(), format!("frc{team}")),
            };
            let value = g.text.fit(&value, &g.ty.label, r.w * 0.55);
            g.row(r, &name, &value, t.dim, i == self.list.focus);
        }
        self.list.scrollbar(g, items.len(), top, bottom);
        g.hint("enter: show as a code");
    }
}

/// A code shown large, with what it is beside it.
pub struct Code {
    title: String,
    qr: qrcodegen::QrCode,
    caption: String,
}

impl Code {
    fn new(title: &str, text: &str, caption: &str) -> Option<Self> {
        let qr = qrcodegen::QrCode::encode_text(text, qrcodegen::QrCodeEcc::Medium).ok()?;
        Some(Code { title: title.to_owned(), qr, caption: caption.to_owned() })
    }
}

/// `qr` in `r`, left-aligned and centred down it: dark squares on white
/// with the quiet margin a reader needs, each square whole pixels.
pub fn draw_qr(g: &mut Gfx, qr: &qrcodegen::QrCode, r: Rect) -> Rect {
    let n = qr.size();
    let cell = (r.w.min(r.h) / (n + 4) as f32).floor().max(1.0);
    let side = cell * (n + 4) as f32;
    let (x0, y0) = ((r.x + g.ox).round(), (r.y + (r.h - side) * 0.5).round());
    g.canvas.rrect(Rect::new(x0, y0, side, side), cell * 1.5, Color::WHITE, 1.0);
    for y in 0..n {
        for x in 0..n {
            if qr.get_module(x, y) {
                g.canvas.fill_rect(Rect::new(x0 + (x + 2) as f32 * cell, y0 + (y + 2) as f32 * cell, cell, cell), Color::hex(0x0d0e12), 1.0);
            }
        }
    }
    Rect::new(x0 - g.ox, y0, side, side)
}

impl Screen for Code {
    fn title(&self) -> &str {
        &self.title
    }

    fn keeps_awake(&self) -> bool {
        true
    }

    fn key(&mut self, key: Key, _cx: &mut Cx) -> Nav {
        match key {
            Key::Back | Key::Enter | Key::Left => Nav::Pop,
            _ => Nav::Stay,
        }
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        let ty = g.ty;
        let top = g.top();
        let room = Rect::new(PAD, top, H - top - 6.0, H - top - 6.0);
        let code = draw_qr(g, &self.qr, room);
        let x = code.right() + 12.0;
        let width = W - PAD - x;
        let lines = crate::help::wrap(g, &self.caption, ty.body, width);
        let line_h = ty.body.size + 4.0;
        let mut y = code.y + 6.0;
        for line in lines.iter().take(((code.h - 30.0) / line_h).max(1.0) as usize) {
            g.text_fit(x, y + line_h * 0.5, line, ty.body, t.ink, width);
            y += line_h;
        }
        g.text_at(x, code.bottom() - 6.0 - ty.caption.size * 0.5, Align::Left, "scan with a phone", ty.caption, t.dim);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONF_TEXT: &str = "ctrl_interface=/run/wpa_supplicant\nupdate_config=1\n\nnetwork={\n\tssid=576f726b73686f70\n\tpsk=\"robots;in:the,pit\"\n}\n\nnetwork={\n\tssid=\"Cafe\"\n\tkey_mgmt=NONE\n}\n\nnetwork={\n\tssid=\"Hashed\"\n\tpsk=8a7f3b2c9d0e1f2a3b4c5d6e7f8091a2b3c4d5e6f708192a3b4c5d6e7f809102\n}\n";

    #[test]
    fn a_saved_network_is_found_by_its_name_however_it_was_written() {
        assert_eq!(saved_secret(CONF_TEXT, "Workshop"), Some(Secret::Password("robots;in:the,pit".into())));
        assert_eq!(saved_secret(CONF_TEXT, "Cafe"), Some(Secret::Open));
        assert_eq!(saved_secret(CONF_TEXT, "Hashed"), Some(Secret::KeyOnly));
        assert_eq!(saved_secret(CONF_TEXT, "Elsewhere"), None);
    }

    #[test]
    fn the_wifi_code_is_what_phones_read_with_its_marks_escaped() {
        assert_eq!(wifi_qr_text("Workshop", &Secret::Password("robots;in:the,pit".into())).unwrap(), r"WIFI:T:WPA;S:Workshop;P:robots\;in\:the\,pit;;");
        assert_eq!(wifi_qr_text("Cafe \"5805\"", &Secret::Open).unwrap(), r#"WIFI:T:nopass;S:Cafe \"5805\";;"#);
        assert_eq!(wifi_qr_text("Hashed", &Secret::KeyOnly), None);
    }

    #[test]
    fn what_is_shared_fits_a_code() {
        assert!(Code::new("Text", &"x".repeat(300), "x").is_some());
        assert!(Code::new("SSH", "ssh://root@10.58.5.77", "ssh root@10.58.5.77").is_some());
    }
}
