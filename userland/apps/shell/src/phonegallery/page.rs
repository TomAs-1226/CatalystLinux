//! The page a phone is given: one HTML file with its style and script in
//! it (a phone on the handheld's hotspot may have no internet, so nothing
//! comes from anywhere else), in the handheld's own look.
//!
//! Its colours are the shell's palette for the accent in use, dark and
//! light (`bezel::Tokens::of`), following the phone's own setting; its
//! letters are the panel's two voices, served by the handheld; its motion
//! is Detent's springs, solved in closed form in the page as the shell
//! solves them, and handed the flick's velocity. The brand's mark keeps its
//! own crimson, as on the panel.
//!
//! The album is newest first, by day, each with its event or places. A
//! photo opens full screen out of its tile; a swipe goes between photos, a
//! pull down puts it away. Under it: its caption, place and match, Save,
//! and Share where the browser allows it (never on plain HTTP: there, a
//! long press on the photo saves or shares it). Select chooses several to
//! save as one ZIP; "Download all" saves the album.

use bezel::{Accent, Color, Tokens, Tone};
use detent::{role, Spring};

use super::serve::Stopped;
use super::Item;

/// What the page may do: its own script and style, pictures, videos and
/// fonts from the handheld, nothing from anywhere else, never framed.
pub const CSP: &str = "default-src 'none'; img-src 'self' data: blob:; media-src 'self' blob:; style-src 'unsafe-inline'; script-src 'unsafe-inline'; font-src 'self'; connect-src 'self'; form-action 'self'; base-uri 'none'; frame-ancestors 'none'";

/// What the page is dressed in, read from the handheld's settings when
/// sharing starts.
#[derive(Clone, Debug)]
pub struct Look {
    pub accent: Accent,
    /// The handheld's name (Settings › Name & time).
    pub name: String,
    pub h24: bool,
}

impl Look {
    pub fn of(settings: &crate::shell::Settings) -> Look {
        Look { accent: settings.accent, name: crate::device::name(), h24: settings.clock24 }
    }
}

fn hex(c: Color) -> String {
    format!("#{:02x}{:02x}{:02x}", c.0, c.1, c.2)
}

fn rgb(c: Color) -> String {
    format!("{} {} {}", c.0, c.1, c.2)
}

/// The palette as custom properties: every colour by its Bezel role.
fn palette_css(t: &Tokens) -> String {
    let roles = [
        ("ground", t.ground),
        ("s1", t.surface1),
        ("s2", t.surface2),
        ("s3", t.surface3),
        ("ink", t.ink),
        ("dim", t.dim),
        ("faint", t.faint),
        ("signal", t.signal),
        ("on-signal", t.on_signal),
        ("ice", t.ice),
        ("on-ice", t.on_ice),
    ];
    let mut css: String = roles.iter().map(|(name, c)| format!("--{name}:{};", hex(*c))).collect();
    // The glass and the shadows mix these in at an opacity.
    for (name, c) in [("ground", t.ground), ("s2", t.surface2), ("ink", t.ink)] {
        css.push_str(&format!("--{name}-rgb:{};", rgb(c)));
    }
    css.push_str(if t.tone == Tone::Light { "--shade:.10;--rim:.55;" } else { "--shade:.34;--rim:.10;" });
    css
}

/// A spring for the page's script: its natural frequency and damping
/// ratio, which is all the closed form needs.
fn spring_js(s: Spring) -> String {
    format!("[{:.3},{:.4}]", s.omega0(), s.damping_ratio())
}

/// A spring as a CSS `linear()` easing and how long it runs, for what
/// moves without a hand on it (an image arriving, the dock tucking away).
fn css_ease(s: Spring) -> (String, f32) {
    // Settled: within 0.1% of the way, and slower than that a second.
    let mut end = 0.0f32;
    let mut t = 0.0f32;
    while t < 3.0 {
        let (x, v) = s.solve(1.0, 0.0, t);
        if x.abs() > 0.001 || v.abs() > 0.01 {
            end = t;
        }
        t += 1.0 / 240.0;
    }
    let end = end.max(0.05);
    const STOPS: usize = 32;
    let points: Vec<String> = (0..=STOPS)
        .map(|i| {
            let p = 1.0 - s.solve(1.0, 0.0, end * i as f32 / STOPS as f32).0;
            let p = if i == STOPS { 1.0 } else { p };
            let s = format!("{p:.4}");
            s.trim_end_matches('0').trim_end_matches('.').to_owned()
        })
        .collect();
    (format!("linear({})", points.join(",")), end)
}

fn springs_json() -> String {
    let named = [("page", role::page()), ("release", role::release()), ("smooth", role::smooth()), ("settle", role::settle()), ("effect", role::effect()), ("edge", role::edge())];
    let fields: Vec<String> = named.iter().map(|(n, s)| format!("\"{n}\":{}", spring_js(*s))).collect();
    format!("{{{}}}", fields.join(","))
}

fn motion_css() -> String {
    let mut css = String::new();
    for (name, s) in [("effect", role::effect()), ("release", role::release()), ("smooth", role::smooth()), ("layer", role::layer())] {
        let (ease, secs) = css_ease(s);
        css.push_str(&format!("--ease-{name}:{ease};--dur-{name}:{secs:.3}s;"));
    }
    css
}

/// Text made safe in HTML.
pub fn escaped(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&#39;")
}

/// JSON made safe inside a `<script>` element.
fn script_safe(json: String) -> String {
    json.replace('<', "\\u003c").replace('>', "\\u003e").replace('&', "\\u0026").replace('\u{2028}', "\\u2028").replace('\u{2029}', "\\u2029")
}

/// The Material Symbols the page uses, as one sprite, from the panel's own
/// icon files (and the two it carries none of).
fn sprite() -> String {
    const FILES: [(&str, &str); 10] = [
        ("download", include_str!("../../../../assets/icons/download.svg")),
        ("place", include_str!("../../../../assets/icons/location_on.svg")),
        ("loop", include_str!("../../../../assets/icons/animation.svg")),
        ("play", include_str!("../../../../assets/icons/play_arrow_fill.svg")),
        ("album", include_str!("../../../../assets/icons/photo_library.svg")),
        ("off", include_str!("../../../../assets/icons/wifi_off.svg")),
        ("match", include_str!("../../../../assets/icons/sports_score.svg")),
        ("claude", include_str!("../../../../assets/icons/auto_awesome_fill.svg")),
        ("broken", include_str!("../../../../assets/icons/broken_image.svg")),
        ("done", include_str!("../../../../assets/icons/download_done.svg")),
    ];
    let mut out = String::from("<svg xmlns=\"http://www.w3.org/2000/svg\" style=\"display:none\">");
    for (id, svg) in FILES {
        let d = svg.split("d=\"").nth(1).and_then(|r| r.split('"').next()).unwrap_or("");
        out.push_str(&format!("<symbol id=\"i-{id}\" viewBox=\"0 -960 960 960\"><path d=\"{d}\"/></symbol>"));
    }
    // Rounded, as the set: close, check, and a share tray.
    out.push_str("<symbol id=\"i-close\" viewBox=\"0 -960 960 960\"><path d=\"M480-429 316-265q-11 11-25 10.5T266-266q-11-11-11-25.5t11-25.5l163-163-164-164q-11-11-10.5-25.5T266-695q11-11 25.5-11t25.5 11l163 164 164-164q11-11 25.5-11t25.5 11q11 11 11 25.5T695-644L531-480l164 164q11 11 11 25t-11 25q-11 11-25.5 11T644-266L480-429Z\"/></symbol>");
    out.push_str("<symbol id=\"i-check\" viewBox=\"0 -960 960 960\"><path d=\"M382-354 721-693q11-11 25.5-11t25.5 11q11 11 11 25.5T772-642L408-277q-11 11-26 11t-26-11L188-445q-11-11-10.5-25.5T189-496q11-11 25.5-11t25.5 11l142 142Z\"/></symbol>");
    out.push_str("<symbol id=\"i-share\" viewBox=\"0 -960 960 960\"><path d=\"M444-784 392-732q-11 11-25.5 11T341-732q-11-11-11-25.5t11-25.5l113-113q11-11 26-11t26 11l113 113q11 11 11 25.5T619-732q-11 11-25.5 11T568-732l-52-52v356q0 15-10.5 25.5T480-392q-15 0-25.5-10.5T444-428v-356ZM264-48q-30 0-51-21t-21-51v-408q0-30 21-51t51-21h84q15 0 25.5 10.5T384-564q0 15-10.5 25.5T348-528h-84v408h432v-408h-84q-15 0-25.5-10.5T576-564q0-15 10.5-25.5T612-600h84q30 0 51 21t21 51v408q0 30-21 51t-51 21H264Z\"/></symbol>");
    out.push_str("</svg>");
    out
}

/// The brand's mark on its ground, as the setup page and the panel draw it.
fn mark_svg(size: u32) -> String {
    let stops: String = crate::brand::GROUND_STOPS.iter().map(|(at, c)| format!("<stop offset=\"{at}\" stop-color=\"{}\"/>", hex(*c))).collect();
    let crimson = hex(crate::brand::CRIMSON);
    format!(
        "<svg class=\"mark\" width=\"{size}\" height=\"{size}\" viewBox=\"0 0 512 512\" aria-hidden=\"true\"><defs><linearGradient id=\"mg\" x1=\"0\" y1=\"0\" x2=\"1\" y2=\"1\">{stops}</linearGradient><radialGradient id=\"ml\"><stop offset=\"0\" stop-color=\"{crimson}\" stop-opacity=\".3\"/><stop offset=\"1\" stop-color=\"{crimson}\" stop-opacity=\"0\"/></radialGradient></defs><rect width=\"512\" height=\"512\" rx=\"114\" fill=\"url(#mg)\"/><circle cx=\"256\" cy=\"256\" r=\"196\" fill=\"url(#ml)\"/><path transform=\"translate(88 88) scale(14)\" d=\"{}\" fill=\"{crimson}\"/></svg>",
        crate::brand::BOLT_PATH
    )
}

/// The mark as the tab's icon.
fn favicon() -> String {
    let svg = mark_svg(64).replace('"', "'").replace('#', "%23").replace('<', "%3C").replace('>', "%3E");
    format!("data:image/svg+xml,{svg}")
}

/// "Today", "Yesterday", "Saturday 21 November" (with the year when it is
/// not this one).
pub fn day_words(day: chrono::NaiveDate, today: chrono::NaiveDate) -> String {
    use chrono::Datelike;
    if day == today {
        "Today".into()
    } else if Some(day) == today.pred_opt() {
        "Yesterday".into()
    } else if day.year() == today.year() {
        day.format("%A %-d %B").to_string()
    } else {
        day.format("%A %-d %B %Y").to_string()
    }
}

/// The album as the page's script reads it.
fn item_values(look: &Look, items: &[Item], today: chrono::NaiveDate) -> Vec<serde_json::Value> {
    use serde_json::json;
    items
        .iter()
        .map(|i| {
            let at = i.photo.taken;
            let a = &i.about;
            // Where: the named place (with the event, if at one), else the
            // event and its venue, else the town (photolog/meta.rs).
            let place = match a.place_line() {
                Some(p) if !a.place.is_empty() && !a.event.is_empty() => format!("{p} · {}", a.event),
                Some(p) => p,
                None => String::new(),
            };
            let by = match a.by.as_str() {
                "" => "",
                "you" => "you",
                _ => "claude",
            };
            let time = if look.h24 { at.format("%H:%M").to_string() } else { at.format("%-I:%M %P").to_string() };
            json!({
                "n": i.photo.name,
                "v": i.photo.bytes,
                "video": i.video(),
                "inline": i.inline(),
                "moving": i.moving(),
                "px": i.pixel(),
                "day": at.date().format("%Y-%m-%d").to_string(),
                "dw": day_words(at.date(), today),
                "time": time,
                "when": crate::photos::when(at, look.h24),
                "cap": a.caption,
                "by": by,
                "place": place,
                "event": a.event,
                "match": a.context,
            })
        })
        .collect()
}

/// `/list.json`: the album as it is now.
pub fn items_json(look: &Look, items: &[Item]) -> String {
    let today = chrono::Local::now().date_naive();
    serde_json::json!({ "items": item_values(look, items, today) }).to_string()
}

fn data_json(look: &Look, items: &[Item], demo: Option<serde_json::Value>) -> String {
    let today = chrono::Local::now().date_naive();
    let data = serde_json::json!({
        "name": look.name,
        "items": item_values(look, items, today),
        "demo": demo.unwrap_or(serde_json::Value::Null),
    });
    script_safe(data.to_string())
}

fn frame(title: &str, look: &Look, style: &str, body: &str) -> String {
    let (dark, light) = (Tokens::of(look.accent, Tone::Dark), Tokens::of(look.accent, Tone::Light));
    FRAME
        .replace("{{title}}", &escaped(title))
        .replace("{{favicon}}", &favicon())
        .replace("{{ground_dark}}", &hex(dark.ground))
        .replace("{{ground_light}}", &hex(light.ground))
        .replace("{{palette_dark}}", &palette_css(&dark))
        .replace("{{palette_light}}", &palette_css(&light))
        .replace("{{crimson}}", &hex(crate::brand::CRIMSON))
        .replace("{{motion}}", &motion_css())
        .replace("{{style}}", style)
        .replace("{{body}}", body)
}

/// The gallery.
pub fn page(look: &Look, items: &[Item]) -> String {
    gallery(look, items, None)
}

/// The gallery as a file to open on a computer, for the screenshots:
/// nothing asked of a handheld, and the state shown given (`select`: the
/// names chosen; `open`: a photo open full screen).
#[cfg(not(target_arch = "aarch64"))]
pub fn sample(look: &Look, items: &[Item], demo: serde_json::Value) -> String {
    gallery(look, items, Some(demo))
}

fn gallery(look: &Look, items: &[Item], demo: Option<serde_json::Value>) -> String {
    let body = GALLERY
        .replace("{{sprite}}", &sprite())
        .replace("{{mark}}", &mark_svg(40))
        .replace("{{name}}", &escaped(&look.name))
        .replace("{{springs}}", &springs_json())
        .replace("{{data}}", &data_json(look, items, demo));
    frame(&format!("Photos · {}", look.name), look, GALLERY_STYLE, &body)
}

/// The way in for a phone that came by the address: the six digits.
pub fn code_page(look: &Look, error: Option<&str>) -> String {
    let error = error.map_or(String::new(), |e| format!("<p class=\"error\" role=\"alert\">{}</p>", escaped(e)));
    let body = CODE_BODY.replace("{{mark}}", &mark_svg(56)).replace("{{name}}", &escaped(&look.name)).replace("{{error}}", &error);
    frame(&format!("Photos · {}", look.name), look, NOTE_STYLE, &body)
}

/// What a phone is told once sharing has stopped.
pub fn closed_page(look: &Look, why: Option<Stopped>) -> String {
    let words = match why {
        Some(Stopped::WrongCodes) => "Too many wrong codes were tried, so the handheld stopped sharing its photos.",
        Some(Stopped::TimeUp) => "Nobody had looked for ten minutes, so the handheld stopped sharing its photos.",
        _ => "The handheld has stopped sharing its photos.",
    };
    let body = CLOSED_BODY.replace("{{mark}}", &mark_svg(56)).replace("{{words}}", words);
    frame(&format!("Photos · {}", look.name), look, NOTE_STYLE, &body)
}

const FRAME: &str = r###"<!doctype html>
<html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1,viewport-fit=cover">
<meta name="color-scheme" content="dark light">
<meta name="theme-color" content="{{ground_dark}}" media="(prefers-color-scheme: dark)">
<meta name="theme-color" content="{{ground_light}}" media="(prefers-color-scheme: light)">
<meta name="referrer" content="no-referrer">
<meta name="format-detection" content="telephone=no">
<title>{{title}}</title>
<link rel="icon" href="{{favicon}}">
<style>
@font-face{font-family:"Google Sans Flex";src:url(f/sans.ttf) format("truetype");font-weight:100 1000;font-stretch:25% 151%;font-display:swap}
@font-face{font-family:"Google Sans Code";src:url(f/code.ttf) format("truetype");font-weight:300 800;font-display:swap}
:root{{{palette_dark}}--crimson:{{crimson}};{{motion}}--sans:"Google Sans Flex",system-ui,-apple-system,"Segoe UI",Roboto,sans-serif;--code:"Google Sans Code",ui-monospace,"SF Mono",Menlo,Consolas,monospace;color-scheme:dark}
@media (prefers-color-scheme:light){:root{{{palette_light}}color-scheme:light}}
*{box-sizing:border-box;-webkit-tap-highlight-color:transparent}
html{background:var(--ground);-webkit-text-size-adjust:100%;text-size-adjust:100%}
body{margin:0;background:var(--ground);color:var(--ink);font:400 16px/1.4 var(--sans);-webkit-font-smoothing:antialiased;min-height:100vh;min-height:100dvh}
button{font:inherit;color:inherit;border:0;background:none;padding:0;margin:0;cursor:pointer}
a{color:inherit;text-decoration:none}
.i{width:20px;height:20px;fill:currentColor;flex:none}
.mono{font-family:var(--code);font-variant-numeric:tabular-nums;text-transform:lowercase}
.mark{display:block;flex:none;border-radius:24%}
{{style}}
</style></head><body>
{{body}}
</body></html>"###;

const NOTE_STYLE: &str = r###"
main{min-height:100vh;min-height:100dvh;display:grid;place-items:center;padding:24px max(20px,env(safe-area-inset-left)) calc(24px + env(safe-area-inset-bottom))}
.card{width:min(100%,400px);background:var(--s1);border-radius:28px;padding:28px 24px 24px;text-align:center}
.card .mark{margin:0 auto 18px}
h1{font-size:26px;line-height:1.15;font-weight:650;letter-spacing:-.02em;margin:0 0 8px}
p{color:var(--dim);margin:0 0 18px;font-size:16px}
form{display:flex;flex-direction:column;gap:12px}
input{width:100%;height:56px;border-radius:16px;border:0;background:var(--s2);color:var(--ink);font:600 26px/1 var(--code);letter-spacing:.3em;text-align:center;padding:0 12px;outline:none;box-shadow:inset 0 0 0 1px rgb(var(--ink-rgb)/.08)}
input:focus{box-shadow:inset 0 0 0 2px var(--signal)}
.go{height:52px;border-radius:26px;background:var(--signal);color:var(--on-signal);font-weight:700;font-size:19px}
.error{color:var(--ink);background:var(--s2);border-radius:14px;padding:10px 14px;font-size:15px}
.small{font-size:13px;margin:16px 0 0}
"###;

const CODE_BODY: &str = r###"<main><section class="card">
{{mark}}
<h1>Photos from {{name}}</h1>
<p>Scan the code on the handheld's screen with your camera, or type the six digits beside it.</p>
{{error}}
<form method="get" action="g" autocomplete="off">
<input name="c" inputmode="numeric" pattern="[0-9 ]{6,7}" maxlength="7" required autofocus autocomplete="one-time-code" aria-label="The six digits on the handheld">
<button class="go" type="submit">Open the photos</button>
</form>
<p class="small mono">photos › p on the handheld</p>
</section></main>"###;

const CLOSED_BODY: &str = r###"<main><section class="card">
{{mark}}
<h1>Sharing has stopped</h1>
<p>{{words}}</p>
<p class="small mono">to share again: photos › p on the handheld</p>
</section></main>"###;

const GALLERY_STYLE: &str = r###"
.top{display:flex;align-items:center;gap:12px;padding:calc(16px + env(safe-area-inset-top)) max(16px,env(safe-area-inset-right)) 4px max(16px,env(safe-area-inset-left));max-width:1180px;margin:0 auto}
.titles{flex:1;min-width:0}
h1{font-size:28px;line-height:1.1;font-weight:650;letter-spacing:-.02em;margin:0}
.sub{color:var(--dim);font-size:13px;margin:3px 0 0;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}
.pill{height:44px;min-width:44px;padding:0 18px;border-radius:22px;background:var(--s2);font-size:16px;font-weight:600;display:inline-flex;align-items:center;justify-content:center;gap:6px;flex:none}
main{padding:0 max(12px,env(safe-area-inset-right)) calc(120px + env(safe-area-inset-bottom)) max(12px,env(safe-area-inset-left));max-width:1180px;margin:0 auto}
.day{margin-top:14px}
.dayhead{display:flex;align-items:flex-end;gap:10px;min-height:52px;padding:0 4px 8px}
.dayhead .words{flex:1;min-width:0}
.dayhead h2{font-size:19px;line-height:1.2;font-weight:620;letter-spacing:-.01em;margin:0}
.where{color:var(--dim);font-size:12px;margin:2px 0 0;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}
.count{color:var(--dim);font-size:12px;flex:none;padding-bottom:1px}
.dsel{display:none;height:32px;padding:0 12px;border-radius:16px;background:var(--s2);font-size:13px;font-weight:600;flex:none}
.sel .dsel{display:inline-flex;align-items:center}
.sel .count{display:none}
.grid{display:grid;grid-template-columns:repeat(auto-fill,minmax(82px,1fr));gap:3px}
@media (min-width:700px){.grid{grid-template-columns:repeat(auto-fill,minmax(132px,1fr));gap:4px}}
.tile{position:relative;display:block;width:100%;aspect-ratio:4/3;border-radius:8px;overflow:hidden;background:var(--s2);isolation:isolate;-webkit-touch-callout:none;-webkit-user-select:none;user-select:none}
@supports not (aspect-ratio:1){.tile{height:0;padding-top:75%}}
.tile img{position:absolute;inset:0;width:100%;height:100%;object-fit:cover;display:block;opacity:0;transition:opacity var(--dur-effect) var(--ease-effect),transform var(--dur-release) var(--ease-release),border-radius var(--dur-release) var(--ease-release)}
.tile img.in{opacity:1}
.px img,.px video{image-rendering:pixelated;image-rendering:crisp-edges}
.tile .glyph{position:absolute;inset:0;display:none;place-items:center;color:var(--faint)}
.tile .glyph .i{width:24px;height:24px}
.tile.nothumb .glyph{display:grid}
.badge{position:absolute;left:4px;bottom:4px;height:20px;padding:0 7px 0 4px;border-radius:10px;background:rgb(0 0 0/.55);color:rgb(240 240 240);font:500 11px/20px var(--code);display:flex;gap:3px;align-items:center;text-transform:lowercase;pointer-events:none}
.badge .i{width:14px;height:14px}
.check{position:absolute;right:5px;top:5px;width:24px;height:24px;border-radius:12px;display:none;place-items:center;border:2px solid rgb(255 255 255/.92);box-shadow:0 0 0 1px rgb(0 0 0/.18),0 1px 4px rgb(0 0 0/.3);transition:background-color var(--dur-effect) var(--ease-effect);pointer-events:none}
.check .i{width:16px;height:16px;fill:var(--on-signal);opacity:0;transition:opacity var(--dur-effect) var(--ease-effect)}
.sel .check{display:grid}
.tile.on .check{background:var(--signal);border-color:var(--signal)}
.tile.on .check .i{opacity:1}
.tile.on img{transform:scale(.86);border-radius:7px}
.press{position:relative;overflow:hidden;transition:transform var(--dur-release) var(--ease-release)}
.press::before{content:"";position:absolute;inset:0;border-radius:inherit;background:radial-gradient(circle at var(--x,50%) var(--y,50%),currentColor,transparent 72%);opacity:0;transition:opacity var(--dur-layer) var(--ease-layer);pointer-events:none;z-index:3}
.press.down::before{opacity:.14}
.press.down{transform:scale(.96)}
.tile.press::before{background:radial-gradient(circle at var(--x,50%) var(--y,50%),rgb(255 255 255/.9),transparent 72%)}
.empty{display:grid;justify-items:center;gap:6px;text-align:center;padding:18vh 24px 0;color:var(--dim)}
.empty .i{width:40px;height:40px;fill:var(--faint);margin-bottom:6px}
.empty h2{color:var(--ink);font-size:20px;font-weight:620;margin:0}
.empty p{margin:0;max-width:300px}
.glass{background:rgb(var(--s2-rgb)/.62);-webkit-backdrop-filter:blur(18px) saturate(1.4);backdrop-filter:blur(18px) saturate(1.4);box-shadow:inset 0 1px 0 rgb(255 255 255/var(--rim)),inset 0 0 0 1px rgb(var(--ink-rgb)/.07),0 12px 32px rgb(0 0 0/var(--shade))}
@media (prefers-reduced-transparency:reduce){.glass{background:var(--s2);-webkit-backdrop-filter:none;backdrop-filter:none;box-shadow:inset 0 0 0 1px var(--s3),0 12px 32px rgb(0 0 0/var(--shade))}}
.dock{position:fixed;z-index:5;left:50%;bottom:calc(16px + env(safe-area-inset-bottom));display:flex;align-items:center;gap:6px;padding:6px;border-radius:32px;max-width:calc(100% - 24px);transform:translate(-50%,0);transition:transform var(--dur-release) var(--ease-release)}
.dock.tucked{transform:translate(-50%,calc(100% + 24px + env(safe-area-inset-bottom)));transition-timing-function:var(--ease-smooth);transition-duration:var(--dur-smooth)}
.dock .part{display:none;align-items:center;gap:6px;min-width:0}
.dock .part.on{display:flex}
.btn{height:52px;border-radius:26px;padding:0 22px;display:inline-flex;align-items:center;justify-content:center;gap:8px;white-space:nowrap;flex:none}
.btn .i{width:22px;height:22px}
.primary{background:var(--signal);color:var(--on-signal);font-weight:700;font-size:19px}
.tonal{background:var(--s3);color:var(--ink);font-weight:600;font-size:17px}
.round{width:52px;padding:0}
.btn .n{font:500 14px/1 var(--code);opacity:.8}
.selN{padding:0 8px;font-weight:600;font-size:16px;white-space:nowrap;min-width:0;overflow:hidden;text-overflow:ellipsis}
[aria-disabled=true]{opacity:.4;pointer-events:none}
.island{position:fixed;z-index:30;top:calc(10px + env(safe-area-inset-top));left:50%;max-width:calc(100% - 32px);display:flex;align-items:center;gap:8px;padding:11px 18px 11px 14px;border-radius:24px;font-size:15px;font-weight:550;transform:translateX(-50%) scale(.2,.4);transform-origin:50% 0;visibility:hidden;transition:transform var(--dur-smooth) var(--ease-smooth),visibility 0s linear var(--dur-smooth)}
.island.on{visibility:visible;transform:translateX(-50%) scale(1);transition:transform var(--dur-release) var(--ease-release),visibility 0s}
.island span{opacity:0;transition:opacity var(--dur-effect) var(--ease-effect)}
.island.on span{opacity:1;transition-delay:calc(var(--dur-release) * .25)}
.island .i{width:20px;height:20px;flex:none}
.viewer{position:fixed;inset:0;z-index:20;display:flex;flex-direction:column;visibility:hidden;overscroll-behavior:contain}
.viewer.open{visibility:visible}
.scrim{position:absolute;inset:0;background:var(--ground);opacity:0}
.stage{position:relative;flex:1;min-height:0;overflow:hidden;touch-action:none}
.track{position:absolute;inset:0;will-change:transform}
.slide{position:absolute;inset:0}
.box{position:absolute;inset:0;transform-origin:50% 50%}
.box img,.box video{position:absolute;inset:0;width:100%;height:100%;object-fit:contain;display:block}
.box .th{filter:blur(3px);transform:scale(1.002)}
.box .full{opacity:0;transition:opacity var(--dur-effect) var(--ease-effect)}
.box .full.in{opacity:1}
.box.loaded .th{opacity:0;transition:opacity var(--dur-effect) var(--ease-effect)}
.vbar{position:absolute;z-index:4;top:calc(10px + env(safe-area-inset-top));left:max(12px,env(safe-area-inset-left));right:max(12px,env(safe-area-inset-right));display:flex;justify-content:space-between;align-items:center;pointer-events:none}
.vbar>*{pointer-events:auto}
.circle{width:44px;height:44px;border-radius:22px;display:grid;place-items:center}
.counter{height:36px;padding:0 14px;border-radius:18px;display:flex;align-items:center;font-size:13px}
.info{position:relative;z-index:3;transform:translate3d(0,calc(var(--rise,0) * 100% + var(--pull,0px)),0);background:var(--s1);border-radius:26px 26px 0 0;padding:16px max(18px,env(safe-area-inset-right)) calc(16px + env(safe-area-inset-bottom)) max(18px,env(safe-area-inset-left));max-height:48vh;overflow-y:auto;overscroll-behavior:contain;will-change:transform}
.when{color:var(--dim);font-size:13px;margin:0}
.cap{font-size:17px;line-height:1.42;margin:6px 0 0;overflow-wrap:anywhere}
.by{display:inline-flex;align-items:center;gap:4px;color:var(--dim);font-size:12px;margin-top:6px}
.by .i{width:14px;height:14px}
.place{display:flex;gap:6px;align-items:flex-start;color:var(--dim);font-size:15px;margin:10px 0 0}
.place .i{width:18px;height:18px;margin-top:1px}
.chips{display:flex;gap:6px;flex-wrap:wrap;margin-top:10px}
.chip{height:30px;border-radius:15px;padding:0 12px 0 9px;background:var(--s3);font:500 13px/30px var(--code);text-transform:lowercase;display:inline-flex;align-items:center;gap:5px}
.chip .i{width:16px;height:16px}
.actions{display:flex;gap:10px;margin-top:14px}
.actions .btn{flex:1}
.note{color:var(--dim);font-size:12px;margin:10px 0 0;text-align:center}
.fly{position:fixed;z-index:25;left:0;top:0;transform-origin:0 0;object-fit:cover;pointer-events:none;will-change:transform}
@media (orientation:landscape) and (max-height:540px){
 .viewer{flex-direction:row}
 .info{width:min(380px,44vw);max-height:none;border-radius:26px 0 0 26px;padding-top:calc(16px + env(safe-area-inset-top))}
}
@media (prefers-reduced-motion:reduce){.press.down{transform:none}.tile.on img{transform:scale(.9)}}
"###;

const GALLERY: &str = r###"{{sprite}}
<header class="top">
{{mark}}
<div class="titles"><h1>Photos</h1><p class="sub mono" id="sub">from {{name}}</p></div>
<button class="pill press" id="selectBtn" type="button" aria-pressed="false">Select</button>
</header>
<main id="days" aria-live="polite"></main>
<div class="dock glass" id="dock">
 <div class="part on" id="dockAll"><a class="btn primary press" id="all" href="zip/all" download><svg class="i"><use href="#i-download"/></svg><span>Download all</span><span class="n" id="allN"></span></a></div>
 <div class="part" id="dockSel"><button class="btn tonal round press" id="clear" type="button" aria-label="Stop selecting"><svg class="i"><use href="#i-close"/></svg></button><span class="selN" id="selN"></span><button class="btn tonal round press" id="shareSel" type="button" aria-label="Share" hidden><svg class="i"><use href="#i-share"/></svg></button><button class="btn primary press" id="saveSel" type="button"><svg class="i"><use href="#i-download"/></svg><span>Save</span></button></div>
</div>
<div class="island glass" id="island" role="status" aria-live="polite"><svg class="i"><use href="#i-done"/></svg><span id="islandWords"></span></div>
<div class="viewer" id="viewer" role="dialog" aria-modal="true" aria-label="Photo">
 <div class="scrim" id="scrim"></div>
 <div class="stage" id="stage"><div class="track" id="track"><div class="slide"></div><div class="slide"></div><div class="slide"></div></div>
  <div class="vbar"><button class="circle glass press" id="close" type="button" aria-label="Close"><svg class="i"><use href="#i-close"/></svg></button><span class="counter glass mono" id="counter"></span></div>
 </div>
 <section class="info" id="info">
  <p class="when mono" id="when"></p>
  <p class="cap" id="cap"></p>
  <span class="by mono" id="by"><svg class="i"><use href="#i-claude"/></svg><span></span></span>
  <p class="place" id="place"><svg class="i"><use href="#i-place"/></svg><span></span></p>
  <div class="chips" id="chips"></div>
  <div class="actions"><a class="btn primary press" id="save" download><svg class="i"><use href="#i-download"/></svg><span>Save</span></a><button class="btn tonal press" id="share" type="button" hidden><svg class="i"><use href="#i-share"/></svg><span>Share</span></button></div>
  <p class="note" id="note"></p>
 </section>
</div>
<script id="data" type="application/json">{{data}}</script>
<script>
(function(){
"use strict";
var D=JSON.parse(document.getElementById("data").textContent);
var SPRINGS={{springs}};
var items=D.items||[];
var demo=D.demo||null;
var calm=window.matchMedia("(prefers-reduced-motion: reduce)");
function $(id){return document.getElementById(id)}
function make(tag,cls,text){var e=document.createElement(tag);if(cls)e.className=cls;if(text!=null)e.textContent=text;return e}
function icon(name,cls){var ns="http://www.w3.org/2000/svg";var s=document.createElementNS(ns,"svg");s.setAttribute("class","i"+(cls?" "+cls:""));var u=document.createElementNS(ns,"use");u.setAttribute("href","#i-"+name);u.setAttributeNS("http://www.w3.org/1999/xlink","xlink:href","#i-"+name);s.appendChild(u);return s}
function enc(n){return encodeURIComponent(n)}
function thumbUrl(it){return "t/"+enc(it.n)+"?v="+it.v}
function fileUrl(it){return "p/"+enc(it.n)+"?v="+it.v}
function saveUrl(it){return "d/"+enc(it.n)}
function plural(n,one,many){return n+" "+(n===1?one:many)}
function mb(bytes){return bytes>=1e9?(bytes/1e9).toFixed(1)+" GB":bytes>=1e6?Math.max(1,Math.round(bytes/1e6))+" MB":Math.max(1,Math.round(bytes/1e3))+" KB"}

/* Detent's springs, solved in closed form as the shell solves them: a
   retarget keeps the velocity it had, so every motion can be turned round. */
function solve(s,x0,v0,t){var w0=s[0],z=s[1];
 if(Math.abs(z-1)<1e-4){var d=v0+w0*x0,e=Math.exp(-w0*t);return [(x0+d*t)*e,(d-w0*(x0+d*t))*e]}
 if(z<1){var a=z*w0,wd=w0*Math.sqrt(1-z*z),c=(v0+a*x0)/wd,e2=Math.exp(-a*t),si=Math.sin(wd*t),co=Math.cos(wd*t);return [e2*(x0*co+c*si),e2*(v0*co-(a*c+x0*wd)*si)]}
 var r=Math.sqrt(z*z-1),r1=-w0*(z-r),r2=-w0*(z+r),A=(v0-r2*x0)/(r1-r2),B=x0-A,e3=Math.exp(r1*t),e4=Math.exp(r2*t);return [A*e3+B*e4,A*r1*e3+B*r2*e4]}
function spring(name){var s=SPRINGS[name];return calm.matches&&s[1]<1?[s[0],1]:s}
function Motion(value,apply,rest){this.value=value;this.vel=0;this.target=value;this.apply=apply;this.rest=rest||0.5;this.raf=0;this.done=null;var m=this;this.step=function(now){m.frame(now)}}
Motion.prototype.to=function(target,name,vel,done){if(vel!=null)this.vel=vel;this.s=spring(name);this.x0=this.value-target;this.v0=this.vel;this.target=target;this.t0=performance.now();this.done=done||null;if(!this.raf)this.raf=requestAnimationFrame(this.step)};
Motion.prototype.frame=function(now){var t=Math.max(0,(now-this.t0)/1000),r=solve(this.s,this.x0,this.v0,t);this.value=this.target+r[0];this.vel=r[1];
 if(Math.abs(r[0])<this.rest&&Math.abs(r[1])<this.rest*12){this.value=this.target;this.vel=0;this.raf=0;this.apply(this.value);var d=this.done;this.done=null;if(d)d();return}
 this.apply(this.value);this.raf=requestAnimationFrame(this.step)};
Motion.prototype.jump=function(v){if(this.raf)cancelAnimationFrame(this.raf);this.raf=0;this.value=this.target=v;this.vel=0;this.done=null;this.apply(v)};
Motion.prototype.moving=function(){return this.raf!==0};

/* A finger's velocity: least squares over its last 100 ms. */
function Tracker(){this.p=[]}
Tracker.prototype.add=function(t,x,y){this.p.push([t,x,y]);while(this.p.length>2&&t-this.p[0][0]>100)this.p.shift()};
Tracker.prototype.v=function(k){var p=this.p,n=p.length;if(n<2)return 0;var mt=0,mx=0,i;for(i=0;i<n;i++){mt+=p[i][0];mx+=p[i][k]}mt/=n;mx/=n;var num=0,den=0;for(i=0;i<n;i++){var dt=p[i][0]-mt;num+=dt*(p[i][k]-mx);den+=dt*dt}return den>0?num/den*1000:0};
/* Where a flick comes to rest (UIScrollView's 0.998). */
function project(v){return v/1000*0.998/(1-0.998)}
function rubber(x,d){var c=0.55;return (1-1/(Math.abs(x)*c/d+1))*d*(x<0?-1:1)}

/* Every control answers on touch: a state layer from where it was touched. */
document.addEventListener("pointerdown",function(e){var b=e.target.closest&&e.target.closest(".press");if(!b)return;var r=b.getBoundingClientRect();b.style.setProperty("--x",(e.clientX-r.left)+"px");b.style.setProperty("--y",(e.clientY-r.top)+"px");b.classList.add("down");
 var up=function(){b.classList.remove("down");window.removeEventListener("pointerup",up);window.removeEventListener("pointercancel",up)};window.addEventListener("pointerup",up);window.addEventListener("pointercancel",up)},{passive:true});

/* Sharing a file, where the browser allows it (a secure page only). */
var canShare=false;try{canShare=!!(navigator.share&&navigator.canShare&&navigator.canShare({files:[new File([new Blob(["x"])],"x.jpg",{type:"image/jpeg"})]}))}catch(e){canShare=false}

/* The island: a message grown out of the top. */
var islandTimer=0,stopped=false;
function say(words,iconName,stay){var isl=$("island");$("islandWords").textContent=words;isl.querySelector("use").setAttribute("href","#i-"+(iconName||"done"));isl.classList.add("on");clearTimeout(islandTimer);if(!stay)islandTimer=setTimeout(function(){isl.classList.remove("on")},3200)}

/* ---- the album, by day ---- */
var selecting=false,chosen=new Set(),days=$("days");
function render(){
 days.textContent="";
 var total=0;items.forEach(function(it){total+=it.v});
 $("sub").textContent="from "+D.name+" · "+plural(items.length,"photo","photos");
 $("allN").textContent=items.length?String(items.length):"";
 $("all").setAttribute("aria-label","Download all "+plural(items.length,"photo","photos")+", "+mb(total));
 if(!items.length){var e=make("div","empty");e.appendChild(icon("album"));e.appendChild(make("h2",null,"No photos yet"));e.appendChild(make("p",null,"Photos taken with the handheld's camera appear here."));days.appendChild(e);$("dock").classList.add("tucked");$("selectBtn").hidden=true;return}
 $("selectBtn").hidden=false;
 var groups=[],at={};
 items.forEach(function(it,i){var g=at[it.day];if(!g){g=at[it.day]={day:it.day,dw:it.dw,list:[]};groups.push(g)}g.list.push(i)});
 groups.forEach(function(g){
  var sec=make("section","day");sec.dataset.day=g.day;
  var head=make("div","dayhead"),words=make("div","words");words.appendChild(make("h2",null,g.dw));
  var where=[];g.list.forEach(function(i){var it=items[i],w=it.event||it.place;if(w&&where.indexOf(w)<0)where.push(w)});
  if(where.length){words.appendChild(make("p","where mono",where.slice(0,3).join(" · ")))}
  head.appendChild(words);head.appendChild(make("span","count mono",plural(g.list.length,"photo","photos")));
  var ds=make("button","dsel press","Select all");ds.type="button";ds.dataset.day=g.day;head.appendChild(ds);
  sec.appendChild(head);
  var grid=make("div","grid");
  g.list.forEach(function(i){grid.appendChild(tile(i))});
  sec.appendChild(grid);days.appendChild(sec)});
 marks()}
function tile(i){
 var it=items[i],b=make("button","tile press"+(it.px?" px":""));b.type="button";b.dataset.i=i;b.dataset.n=it.n;
 b.setAttribute("aria-label",(it.video?"Video":it.moving?"Loop":"Photo")+", "+it.dw+", "+it.time+(it.cap?". "+it.cap:""));
 var g=make("span","glyph");g.appendChild(icon(it.moving?"play":"broken"));b.appendChild(g);
 var img=new Image();img.alt="";img.decoding="async";img.loading="lazy";img.width=96;img.height=72;
 img.onload=function(){img.classList.add("in")};img.onerror=function(){b.classList.add("nothumb");img.remove()};img.src=thumbUrl(it);b.appendChild(img);
 if(it.moving){var bd=make("span","badge");bd.appendChild(icon(it.video?"play":"loop"));bd.appendChild(make("span",null,it.video?"video":"loop"));b.appendChild(bd)}
 var c=make("span","check");c.appendChild(icon("check"));b.appendChild(c);
 return b}
function marks(){
 var tiles=days.querySelectorAll(".tile");for(var k=0;k<tiles.length;k++){var t=tiles[k],on=chosen.has(t.dataset.n);t.classList.toggle("on",on);t.setAttribute("aria-pressed",selecting?String(on):"false")}
 var secs=days.querySelectorAll(".day");for(var s=0;s<secs.length;s++){var ts=secs[s].querySelectorAll(".tile"),all=ts.length>0;for(var q=0;q<ts.length;q++)if(!chosen.has(ts[q].dataset.n)){all=false;break}var ds=secs[s].querySelector(".dsel");if(ds)ds.textContent=all?"Deselect":"Select all"}
 var n=chosen.size,bytes=0;items.forEach(function(it){if(chosen.has(it.n))bytes+=it.v});
 $("selN").textContent=n?plural(n,"photo","photos")+" · "+mb(bytes):"Choose photos";
 $("saveSel").setAttribute("aria-disabled",String(n===0||stopped));$("shareSel").hidden=!canShare;$("shareSel").setAttribute("aria-disabled",String(n===0||stopped))}

/* The dock: away while the album scrolls down, back as it scrolls up. */
var dock=$("dock"),lastY=window.scrollY,dockSwap=0;
window.addEventListener("scroll",function(){var y=window.scrollY,dy=y-lastY;lastY=y;if(V.open||!items.length)return;if(dy>6&&y>40)dock.classList.add("tucked");else if(dy<-6||y<40)dock.classList.remove("tucked")},{passive:true});
function setSelecting(on){if(selecting===on)return;selecting=on;if(!on)chosen.clear();document.body.classList.toggle("sel",on);$("selectBtn").textContent=on?"Done":"Select";$("selectBtn").setAttribute("aria-pressed",String(on));
 /* The dock steps away and comes back carrying the other controls. */
 dock.classList.add("tucked");clearTimeout(dockSwap);dockSwap=setTimeout(function(){$("dockAll").classList.toggle("on",!on);$("dockSel").classList.toggle("on",on);if(!stopped)dock.classList.remove("tucked")},calm.matches?0:150);marks()}
$("selectBtn").addEventListener("click",function(){setSelecting(!selecting)});
$("clear").addEventListener("click",function(){setSelecting(false)});
var pressTimer=0,longPressed=false,pressAt=null;
days.addEventListener("pointerdown",function(e){var t=e.target.closest(".tile");if(!t)return;longPressed=false;pressAt=[e.clientX,e.clientY];clearTimeout(pressTimer);pressTimer=setTimeout(function(){longPressed=true;if(!selecting)setSelecting(true);toggle(t.dataset.n);if(navigator.vibrate)navigator.vibrate(8)},480)});
days.addEventListener("pointermove",function(e){if(pressAt&&Math.hypot(e.clientX-pressAt[0],e.clientY-pressAt[1])>8)clearTimeout(pressTimer)});
["pointerup","pointercancel"].forEach(function(n){days.addEventListener(n,function(){clearTimeout(pressTimer);pressAt=null})});
days.addEventListener("contextmenu",function(e){if(e.target.closest(".tile"))e.preventDefault()});
days.addEventListener("click",function(e){
 var ds=e.target.closest(".dsel");if(ds){var ts=ds.closest(".day").querySelectorAll(".tile"),all=true,k;for(k=0;k<ts.length;k++)if(!chosen.has(ts[k].dataset.n))all=false;for(k=0;k<ts.length;k++){if(all)chosen.delete(ts[k].dataset.n);else chosen.add(ts[k].dataset.n)}marks();return}
 var t=e.target.closest(".tile");if(!t)return;if(longPressed){longPressed=false;return}
 if(selecting){toggle(t.dataset.n);return}
 openAt(+t.dataset.i,t)});
function toggle(n){if(chosen.has(n))chosen.delete(n);else chosen.add(n);marks()}

/* Saving: one file as itself, several as one ZIP made as it is sent. */
function fetchTo(url){var a=make("a");a.href=url;a.setAttribute("download","");document.body.appendChild(a);a.click();a.remove()}
$("all").addEventListener("click",function(e){if(stopped){e.preventDefault();return}var bytes=0;items.forEach(function(it){bytes+=it.v});say(plural(items.length,"photo","photos")+" on the way · "+mb(bytes),"download")});
$("saveSel").addEventListener("click",function(){var names=items.filter(function(it){return chosen.has(it.n)}).map(function(it){return it.n});if(!names.length||stopped)return;
 if(names.length===1){fetchTo(saveUrl(items.find(function(it){return it.n===names[0]})));say("1 photo on the way","download");return}
 fetch("select",{method:"POST",headers:{"Content-Type":"text/plain"},body:names.join("\n"),credentials:"same-origin"}).then(function(r){if(!r.ok)throw r;var b=0;items.forEach(function(it){if(chosen.has(it.n))b+=it.v});fetchTo("zip/selected");say(plural(names.length,"photo","photos")+" on the way · "+mb(b),"download")}).catch(function(){lost()})});
function shareFiles(list){return Promise.all(list.map(function(it){return fetch(fileUrl(it)).then(function(r){return r.blob()}).then(function(b){return new File([b],it.n,{type:b.type})})})).then(function(files){return navigator.share({files:files})}).catch(function(){})}
$("shareSel").addEventListener("click",function(){var list=items.filter(function(it){return chosen.has(it.n)});if(list.length)shareFiles(list)});

/* ---- one photo, full screen ---- */
var V={open:false,i:0,w:1,h:1};
var viewer=$("viewer"),stage=$("stage"),track=$("track"),scrim=$("scrim"),info=$("info");
var slides=[].slice.call(track.children);
var dx=new Motion(0,function(v){track.style.transform="translate3d("+v+"px,0,0)"},0.4);
var pull=new Motion(0,function(v){var b=current();if(!b)return;var k=1-Math.min(Math.max(v,0)/(V.h*2.4),0.25);b.style.transform="translate3d(0,"+v+"px,0) scale("+k+")";scrim.style.opacity=String(Math.max(0,1-Math.max(v,0)/(V.h*0.7)));info.style.setProperty("--pull",Math.max(0,v)*0.6+"px")},0.4);
var fade=new Motion(0,function(v){scrim.style.opacity=String(v)},0.002);
var rise=new Motion(1,function(v){info.style.setProperty("--rise",String(v))},0.002);
function current(){return slides[1].firstChild}
function place(){slides.forEach(function(s,k){s.style.transform="translate3d("+((k-1)*100)+"%,0,0)"})}
function fill(slide,i){slide.textContent="";slide.dataset.i=i;var it=items[i];if(!it)return;
 var box=make("div","box"+(it.px?" px":""));var th=new Image();th.className="th";th.alt="";th.src=thumbUrl(it);th.onerror=function(){th.remove()};box.appendChild(th);slide.appendChild(box)}
function full(slide){var it=items[+slide.dataset.i],box=slide.firstChild;if(!it||!box||box.querySelector(".full"))return;var f;
 if(it.video&&it.inline){f=make("video","full");f.muted=true;f.loop=true;f.playsInline=true;f.setAttribute("playsinline","");f.setAttribute("muted","");f.preload="auto";f.poster=thumbUrl(it);f.onloadeddata=function(){f.classList.add("in");box.classList.add("loaded")};f.src=fileUrl(it)}
 else{f=new Image();f.className="full";f.alt=it.cap||"";f.decoding="async";f.onload=function(){f.classList.add("in");box.classList.add("loaded");if(slide===slides[1])neighbours()};f.src=it.video?thumbUrl(it):fileUrl(it)}
 box.appendChild(f)}
function neighbours(){full(slides[0]);full(slides[2])}
function playOnly(){slides.forEach(function(s,k){var v=s.querySelector("video");if(!v)return;if(k===1){var p=v.play();if(p&&p.catch)p.catch(function(){})}else v.pause()})}
function show(i){V.i=i;fill(slides[0],i-1);fill(slides[1],i);fill(slides[2],i+1);place();dx.jump(0);full(slides[1]);describe();playOnly()}
function describe(){var it=items[V.i];if(!it)return;
 $("counter").textContent=(V.i+1)+" / "+items.length;
 $("when").textContent=it.when;
 var cap=$("cap");cap.textContent=it.cap;cap.hidden=!it.cap;
 var by=$("by");by.hidden=!it.cap||!it.by;by.lastChild.textContent=it.by==="you"?"written by you":"by claude";
 var pl=$("place");pl.hidden=!it.place;pl.lastChild.textContent=it.place;
 var ch=$("chips");ch.textContent="";if(it.match){var c=make("span","chip");c.appendChild(icon("match"));c.appendChild(make("span",null,it.match));ch.appendChild(c)}ch.hidden=!it.match;
 var s=$("save");s.href=saveUrl(it);s.setAttribute("aria-label","Save "+it.n);
 $("share").hidden=!canShare;
 $("note").textContent=canShare?"":it.video?"Saved, it's in your downloads to share from there.":"Or press and hold the photo to save it or share it.";$("note").hidden=canShare}
/* Turning: the new photo becomes the middle slide where it already is,
   then springs home with the flick's velocity. */
function turn(by,vel){var to=V.i+by;if(to<0||to>=items.length){dx.to(0,"page",vel);return}
 V.i=to;if(by>0){slides.push(slides.shift());fill(slides[2],to+1)}else{slides.unshift(slides.pop());fill(slides[0],to-1)}
 place();dx.jump(dx.value+by*V.w);dx.to(0,"page",vel);full(slides[1]);if(slides[1].querySelector(".full.in"))neighbours();describe();playOnly();
 if(history.state&&history.state.viewer)history.replaceState({viewer:1,n:items[to].n},"")}
function measure(){var r=stage.getBoundingClientRect();V.w=r.width||window.innerWidth;V.h=r.height||window.innerHeight;V.left=r.left;V.top=r.top}
/* Where a picture of this shape sits in the stage. */
function fitted(aspect){var w=V.w,h=V.h,fw=w,fh=w/aspect;if(fh>h){fh=h;fw=h*aspect}return {x:V.left+(w-fw)/2,y:V.top+(h-fh)/2,w:fw,h:fh}}
function aspectOf(){var b=current(),f=b&&b.querySelector(".full.in");if(f){var nw=f.naturalWidth||f.videoWidth,nh=f.naturalHeight||f.videoHeight;if(nw&&nh)return nw/nh}return 4/3}
/* A tile's picture flying to (or from) where the photo sits. */
function fly(from,to,thumb,name,done,radius){var e=new Image();e.className="fly"+(items[V.i]&&items[V.i].px?" px":"");e.src=thumb;e.alt="";e.style.width=to.w+"px";e.style.height=to.h+"px";document.body.appendChild(e);
 var m=new Motion(0,function(p){var x=from.x+(to.x-from.x)*p,y=from.y+(to.y-from.y)*p,sx=(from.w+(to.w-from.w)*p)/to.w,sy=(from.h+(to.h-from.h)*p)/to.h;e.style.transform="translate3d("+x+"px,"+y+"px,0) scale("+sx+","+sy+")";e.style.borderRadius=(radius*(1-p))/Math.max(sx,0.01)+"px"},0.001);
 m.jump(0);m.to(1,name,0,function(){done(e)});return m}
function tileOf(i){return days.querySelector('.tile[data-i="'+i+'"]')}
function rectOf(t){var r=t.getBoundingClientRect();return {x:r.left,y:r.top,w:r.width,h:r.height}}
function openAt(i,t,still){if(V.open)return;V.open=true;viewer.classList.add("open");document.documentElement.style.overflow="hidden";dock.classList.add("tucked");measure();show(i);
 if(!(history.state&&history.state.viewer))history.pushState({viewer:1,n:items[i].n},"");
 var b=current();if(still){fade.jump(1);rise.jump(0);return}if(calm.matches||!t){fade.jump(0);fade.to(1,"effect");rise.jump(1);rise.to(0,"release");return}
 var from=rectOf(t);b.style.visibility="hidden";fade.jump(0);fade.to(1,"effect");rise.jump(1);rise.to(0,"release");
 /* The info takes its height first, so the photo lands where it stays. */
 requestAnimationFrame(function(){measure();fly(from,fitted(4/3),thumbUrl(items[i]),"release",function(e){b.style.visibility="";e.remove()},8)})}
function closeViewer(){if(!V.open)return;V.open=false;var t=tileOf(V.i),b=current();
 /* From where a pull left the ground, not from whole. */
 fade.jump(parseFloat(scrim.style.opacity||"1"));
 var end=function(){viewer.classList.remove("open");document.documentElement.style.overflow="";track.style.transform="";slides.forEach(function(s){s.textContent=""});pull.jump(0);info.style.setProperty("--pull","0px");if(!stopped&&items.length)dock.classList.remove("tucked")};
 rise.to(1,"smooth");
 if(t&&!calm.matches){var r=t.getBoundingClientRect();if(r.bottom>0&&r.top<window.innerHeight){
  measure();var at=fitted(aspectOf()),p=pull.value;if(p){var k=1-Math.min(Math.max(p,0)/(V.h*2.4),0.25);at={x:at.x+at.w*(1-k)/2,y:at.y+p+at.h*(1-k)/2,w:at.w*k,h:at.h*k}}
  if(b)b.style.visibility="hidden";fade.to(0,"smooth");fly(at,rectOf(t),thumbUrl(items[V.i]),"smooth",function(e){e.remove();end()},0);return}}
 fade.to(0,"smooth",null,end)}
window.addEventListener("popstate",function(){if(V.open)closeViewer()});
function back(){if(history.state&&history.state.viewer)history.back();else closeViewer()}
$("close").addEventListener("click",back);
document.addEventListener("keydown",function(e){if(!V.open)return;if(e.key==="Escape")back();else if(e.key==="ArrowRight")turn(1,0);else if(e.key==="ArrowLeft")turn(-1,0)});
window.addEventListener("resize",function(){if(V.open){measure();dx.jump(0)}});
$("share").addEventListener("click",function(){var it=items[V.i];if(it)shareFiles([it])});

/* Swipes: sideways between photos, down to put it away. Each follows the
   finger 1:1 after 10 px, and hands its velocity to the spring. */
var G=null;
stage.addEventListener("pointerdown",function(e){if(!V.open||e.target.closest(".vbar"))return;if(G&&G.id!==e.pointerId){G.multi=true;return}
 G={id:e.pointerId,x:e.clientX,y:e.clientY,mode:null,base:dx.value,pull:pull.value,tr:new Tracker()};G.tr.add(e.timeStamp,e.clientX,e.clientY);
 if(dx.moving())dx.jump(dx.value);if(pull.moving())pull.jump(pull.value);try{stage.setPointerCapture(e.pointerId)}catch(_){}});
stage.addEventListener("pointermove",function(e){if(!G||G.id!==e.pointerId||G.multi)return;var mx=e.clientX-G.x,my=e.clientY-G.y;G.tr.add(e.timeStamp,e.clientX,e.clientY);
 if(!G.mode){if(Math.hypot(mx,my)<10)return;G.mode=Math.abs(mx)>=Math.abs(my)?"x":(my>0||G.pull>0?"y":"none");G.x+=Math.sign(mx)*(G.mode==="x"?10:0);G.y+=Math.sign(my)*(G.mode==="y"?10:0);mx=e.clientX-G.x;my=e.clientY-G.y}
 if(G.mode==="x"){var x=G.base+mx;if((V.i===0&&x>0)||(V.i===items.length-1&&x<0))x=rubber(x,V.w);dx.jump(x)}
 else if(G.mode==="y"){pull.jump(Math.max(G.pull+my,rubber(Math.min(G.pull+my,0),80)))}});
function letGo(e){if(!G||G.id!==e.pointerId)return;var g=G;G=null;if(g.multi)return;
 if(g.mode==="x"){var v=g.tr.v(1),end=dx.value+project(v);if(end<-V.w/2&&V.i<items.length-1)turn(1,v);else if(end>V.w/2&&V.i>0)turn(-1,v);else dx.to(0,"page",v)}
 else if(g.mode==="y"){var vy=g.tr.v(2);if(pull.value+project(vy)>Math.min(160,V.h*0.22))back();else pull.to(0,"release",vy)}}
stage.addEventListener("pointerup",letGo);stage.addEventListener("pointercancel",letGo);

/* ---- still sharing? ---- */
var misses=0;
function lost(){stopped=true;say("Sharing stopped on the handheld","off",true);dock.classList.add("tucked");marks()}
function ping(){if(demo||stopped||document.hidden)return;fetch("ping",{cache:"no-store",credentials:"same-origin"}).then(function(r){if(r.status===403||r.status===410)lost();else misses=0}).catch(function(){if(++misses>=2)lost()})}
setInterval(ping,15000);
document.addEventListener("visibilitychange",function(){if(document.hidden||demo||stopped)return;ping();
 fetch("list.json",{cache:"no-store",credentials:"same-origin"}).then(function(r){return r.ok?r.json():null}).then(function(d){if(!d||!d.items)return;var a=d.items.map(function(it){return it.n+it.v}).join("|"),b=items.map(function(it){return it.n+it.v}).join("|");if(a===b||V.open)return;
  items=d.items;var keep=new Set();items.forEach(function(it){if(chosen.has(it.n))keep.add(it.n)});chosen=keep;render()}).catch(function(){})});

render();
if(demo&&demo.select){setSelecting(true);demo.select.forEach(function(n){chosen.add(n)});clearTimeout(dockSwap);$("dockAll").classList.remove("on");$("dockSel").classList.add("on");dock.classList.remove("tucked");marks()}
if(demo&&demo.open!=null){openAt(demo.open,null,true)}
if(demo&&demo.say){say(demo.say,demo.icon||"download",true)}
})();
</script>"###;
