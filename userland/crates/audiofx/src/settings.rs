//! Music's sound settings (Settings › Sound), as the shell writes them and
//! the player reads them: a few `name=value` lines in /data/catalyst. The
//! player looks at the file's time twice a second and reads it again when
//! it changes, so a switch takes effect while a song plays.

use std::path::Path;

use crate::eq::Preset;

pub const PATH: &str = "/data/catalyst/audio-fx";

#[derive(Clone, Debug, PartialEq)]
pub struct Fx {
    /// Spatial sound for headphones (and the AirPods' head tracking).
    pub spatial: bool,
    pub eq: Preset,
    /// The custom preset's bands, in decibels.
    pub bands: [i8; 5],
    /// Every song at the same loudness.
    pub loudness: bool,
    /// Seconds songs overlap by (0: gapless).
    pub crossfade: u8,
}

impl Default for Fx {
    fn default() -> Self {
        Fx { spatial: false, eq: Preset::Flat, bands: [0; 5], loudness: true, crossfade: 0 }
    }
}

pub fn parse(text: &str) -> Fx {
    let mut fx = Fx::default();
    for line in text.lines() {
        let Some((key, value)) = line.split_once('=') else { continue };
        let value = value.trim();
        match key.trim() {
            "spatial" => fx.spatial = value == "on",
            "eq" => fx.eq = Preset::parse(value).unwrap_or(Preset::Flat),
            "bands" => {
                for (band, v) in fx.bands.iter_mut().zip(value.split(',')) {
                    *band = v.trim().parse::<i8>().unwrap_or(0).clamp(-crate::eq::LIMIT, crate::eq::LIMIT);
                }
            }
            "loudness" => fx.loudness = value != "off",
            "crossfade" => fx.crossfade = value.parse::<u8>().unwrap_or(0).min(crate::crossfade::MOST),
            _ => {}
        }
    }
    fx
}

pub fn format(fx: &Fx) -> String {
    let on = |b: bool| if b { "on" } else { "off" };
    let bands: Vec<String> = fx.bands.iter().map(|b| b.to_string()).collect();
    format!("spatial={}\neq={}\nbands={}\nloudness={}\ncrossfade={}\n", on(fx.spatial), fx.eq.word(), bands.join(","), on(fx.loudness), fx.crossfade)
}

pub fn load(path: &Path) -> Fx {
    std::fs::read_to_string(path).map(|t| parse(&t)).unwrap_or_default()
}

/// Written whole (a new file put in place), so the player never reads half.
pub fn save(path: &Path, fx: &Fx) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("new");
    std::fs::write(&tmp, format(fx))?;
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_go_to_the_file_and_back() {
        let fx = Fx { spatial: true, eq: Preset::Custom, bands: [3, -2, 0, 1, 12], loudness: false, crossfade: 5 };
        assert_eq!(parse(&format(&fx)), fx);
        assert_eq!(parse(""), Fx::default());
        // Out of range or garbled: held in bounds, or the default.
        let odd = parse("crossfade=99\nbands=40,x\neq=wobbly\nspatial=yes\n");
        assert_eq!((odd.crossfade, odd.bands[0], odd.bands[1], odd.eq, odd.spatial), (8, 12, 0, Preset::Flat, false));
        let dir = std::env::temp_dir().join(format!("audiofx-settings-{}", std::process::id()));
        let path = dir.join("audio-fx");
        save(&path, &fx).unwrap();
        assert_eq!(load(&path), fx);
        std::fs::remove_dir_all(dir).ok();
        assert_eq!(load(Path::new("/nowhere/audio-fx")), Fx::default());
    }
}
