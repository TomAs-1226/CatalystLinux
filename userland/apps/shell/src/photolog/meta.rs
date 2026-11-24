//! What is said about a photo, kept beside it in an XMP sidecar
//! (`IMG_20261121_140312.xmp`), the way Lightroom, darktable and digiKam
//! keep theirs: the photo itself is never written to, so it stays exactly
//! the JPEG the camera made, and a RAW kept beside it (`.dng`, the same
//! name) shares the sidecar, as those programs expect.
//!
//! The caption is `dc:description`, which they all show; the named place
//! (or the event's venue) `Iptc4xmpCore:Location`, the town
//! `photoshop:City`, the event `Iptc4xmpExt:Event`. What only Catalyst
//! uses (the match, who wrote the caption, the venue on its own) is in its
//! own namespace. The Wi-Fi a photo was taken by is not here: it stays in
//! the handheld's own records (store.rs), since a sidecar goes wherever the
//! photo is copied.

use std::path::{Path, PathBuf};

/// What a photo's sidecar says.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct About {
    pub caption: String,
    /// Who wrote it: the model's name, "you", or empty.
    pub by: String,
    /// The named place.
    pub place: String,
    pub event: String,
    /// The event's venue, as the schedule has it: "Rochester, NY".
    pub venue: String,
    /// Among the team's matches: "during Q31".
    pub context: String,
    /// The town, found from the Wi-Fi: "Henrietta, NY".
    pub town: String,
}

impl About {
    /// Where it was taken, in a line: the named place, else the event and
    /// its venue, else the town.
    pub fn place_line(&self) -> Option<String> {
        if !self.place.is_empty() {
            return Some(self.place.clone());
        }
        match (self.event.is_empty(), self.venue.is_empty()) {
            (false, false) => return Some(format!("{} · {}", self.event, self.venue)),
            (false, true) => return Some(self.event.clone()),
            (true, false) => return Some(self.venue.clone()),
            _ => {}
        }
        (!self.town.is_empty()).then(|| self.town.clone())
    }

    pub fn is_empty(&self) -> bool {
        *self == About::default()
    }

    /// The sidecar's text: an XMP packet.
    pub fn to_xmp(&self) -> String {
        let mut fields = String::new();
        let mut alt = |name: &str, text: &str| {
            if !text.is_empty() {
                fields.push_str(&format!("   <{name}><rdf:Alt><rdf:li xml:lang=\"x-default\">{}</rdf:li></rdf:Alt></{name}>\n", escape(text)));
            }
        };
        alt("dc:description", &self.caption);
        alt("Iptc4xmpExt:Event", &self.event);
        let mut plain = |name: &str, text: &str| {
            if !text.is_empty() {
                fields.push_str(&format!("   <{name}>{}</{name}>\n", escape(text)));
            }
        };
        let location = if self.place.is_empty() { &self.venue } else { &self.place };
        plain("Iptc4xmpCore:Location", location);
        plain("photoshop:City", self.town.split(", ").next().unwrap_or(""));
        plain("catalyst:Place", &self.place);
        plain("catalyst:Venue", &self.venue);
        plain("catalyst:Match", &self.context);
        plain("catalyst:Town", &self.town);
        plain("catalyst:CaptionBy", &self.by);
        format!(
            "<?xpacket begin=\"\u{feff}\" id=\"W5M0MpCehiHzreSzNTczkc9d\"?>\n\
             <x:xmpmeta xmlns:x=\"adobe:ns:meta/\" x:xmptk=\"Catalyst Linux\">\n \
             <rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\">\n  \
             <rdf:Description rdf:about=\"\"\n    \
             xmlns:dc=\"http://purl.org/dc/elements/1.1/\"\n    \
             xmlns:photoshop=\"http://ns.adobe.com/photoshop/1.0/\"\n    \
             xmlns:Iptc4xmpCore=\"http://iptc.org/std/Iptc4xmpCore/1.0/xmlns/\"\n    \
             xmlns:Iptc4xmpExt=\"http://iptc.org/std/Iptc4xmpExt/2008-02-29/\"\n    \
             xmlns:catalyst=\"urn:catalyst-linux:photolog:1\">\n\
             {fields}  </rdf:Description>\n </rdf:RDF>\n</x:xmpmeta>\n<?xpacket end=\"w\"?>\n"
        )
    }

    /// A sidecar read back: what Catalyst wrote, or what another program
    /// wrote in the same fields (as elements or as attributes).
    pub fn from_xmp(text: &str) -> About {
        let caption = field(text, "dc:description");
        let catalyst_place = field(text, "catalyst:Place");
        let venue = field(text, "catalyst:Venue");
        // Another program's location, where Catalyst named none.
        let place = if catalyst_place.is_empty() && venue.is_empty() { field(text, "Iptc4xmpCore:Location") } else { catalyst_place };
        let town = Some(field(text, "catalyst:Town")).filter(|t| !t.is_empty()).unwrap_or_else(|| field(text, "photoshop:City"));
        About { caption, by: field(text, "catalyst:CaptionBy"), place, event: field(text, "Iptc4xmpExt:Event"), venue, context: field(text, "catalyst:Match"), town }
    }
}

/// A photo's sidecar: its name with `.xmp`.
pub fn sidecar(photo: &Path) -> PathBuf {
    photo.with_extension("xmp")
}

/// What a photo's sidecar says (nothing, if it has none).
pub fn read(photo: &Path) -> About {
    std::fs::read_to_string(sidecar(photo)).map(|t| About::from_xmp(&t)).unwrap_or_default()
}

/// Write a photo's sidecar (or remove it, when there is nothing to say).
pub fn write(photo: &Path, about: &About) -> std::io::Result<()> {
    let path = sidecar(photo);
    if about.is_empty() {
        return match std::fs::remove_file(&path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        };
    }
    crate::sys::write_atomic(&path, about.to_xmp().as_bytes())
}

fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            // Kept as written: a line break in a caption.
            '\n' => out.push_str("&#10;"),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out
}

fn unescape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        let Some(end) = rest.find(';').filter(|e| *e <= 10) else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let entity = &rest[1..end];
        let c = match entity {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            e if e.starts_with("#x") || e.starts_with("#X") => u32::from_str_radix(&e[2..], 16).ok().and_then(char::from_u32),
            e if e.starts_with('#') => e[1..].parse().ok().and_then(char::from_u32),
            _ => None,
        };
        match c {
            Some(c) => {
                out.push(c);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// A field's text: an element's (the first `rdf:li` of a list), or an
/// attribute's on `rdf:Description`.
fn field(text: &str, name: &str) -> String {
    let open = format!("<{name}");
    let mut from = 0;
    while let Some(at) = text[from..].find(&open).map(|a| a + from) {
        let after = &text[at + open.len()..];
        // `<dc:descriptionX` is another element.
        if !after.starts_with(['>', ' ', '\n', '\r', '\t', '/']) {
            from = at + open.len();
            continue;
        }
        let Some(gt) = after.find('>') else { break };
        if after[..gt].ends_with('/') {
            return String::new();
        }
        let body = &after[gt + 1..];
        let close = format!("</{name}>");
        let Some(end) = body.find(&close) else { break };
        let inner = &body[..end];
        return match inner.find("<rdf:li") {
            Some(li) => {
                let li = &inner[li..];
                let start = li.find('>').map_or(li.len(), |g| g + 1);
                let stop = li.find("</rdf:li>").unwrap_or(li.len());
                unescape(li.get(start..stop).unwrap_or(""))
            }
            None => unescape(inner.trim()),
        };
    }
    // As an attribute: ` name="…"`.
    for quote in ['"', '\''] {
        let attr = format!("{name}={quote}");
        if let Some(at) = text.find(&attr) {
            let rest = &text[at + attr.len()..];
            if let Some(end) = rest.find(quote) {
                return unescape(&rest[..end]);
            }
        }
    }
    String::new()
}
