//! The Camera's pro mode and the sums behind every photo's exposure.
//!
//! What the camera program says of each preview frame (its exposure time,
//! analogue and digital gain, colour gains, colour temperature and the
//! light) is read here ([`Metered`]), and what a photo is taken with is
//! worked out from it ([`still`]): the preview's settled exposure handed to
//! the still as fixed values, so the camera never has to settle again after
//! changing to the full sensor.
//!
//! Pro mode's dials, each on a key in the Camera:
//!
//! - shutter (`s`): auto, or 1/2000 to 1 s in thirds of a stop;
//! - ISO (`i`): auto, or 100 to 800 in thirds, the sensor's analogue gain
//!   (ISO 100 is a gain of 1, as Raspberry Pi's own camera tools count it);
//! - exposure compensation (`e`): −3 to +3 in thirds (with both of the
//!   above set by hand there is nothing for it to move, and it says so);
//! - white balance (`w`): auto, the usual presets, or a colour temperature
//!   in kelvin, given to libcamera as its `ColourTemperature`.
//!
//! The aperture is the lens's own f/2.0: fixed, so shown and never a dial.
//!
//! The preview keeps to its frame rate whatever the shutter: a shutter
//! longer than a preview frame is shown at the frame's length with the gain
//! raised to match (as cameras simulate a long exposure in the viewfinder),
//! and only the photo takes the whole time.
//!
//! Photos in auto split their exposure for a hand-held camera: the shutter
//! is kept short (1/100 s, then 1/60, then 1/40) by raising the gain first,
//! and goes past 1/30 s only once the gain is spent, because a photo blurred
//! by a moving hand is never crisp however little noise it has. The sensor's
//! own noise filter and sharpening follow the gain the photo ends up with.
//!
//! The histogram, the zebras (highlights at 98% and over, any channel) and
//! focus peaking are worked out here too, from the camera's own picture
//! before any look is laid over it: they are about what the sensor caught.

/// A shutter speed: how it is written, and how long it is (microseconds).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shutter {
    pub label: &'static str,
    pub us: u32,
}

const fn shutter(label: &'static str, us: u32) -> Shutter {
    Shutter { label, us }
}

/// The IMX219's longest exposure at its full size, by its kernel driver
/// (drivers/media/i2c/imx219.c): a frame of at most 0xfffe lines, exposing
/// up to four fewer, each line at most 0x7ff0 pixels at 182.4 million a
/// second (two CSI-2 lanes): 65530 × 32752 / 182.4 MHz, 11.77 s.
/// catalyst-camera's still.h has the same, and holds a photo to what the
/// pipeline says the mode allows.
pub const LONGEST_US: u32 = 11_766_000;

/// The shutter dial: 1/2000 to 10 s in thirds of a stop, as cameras mark
/// them, and the sensor's longest at the end (0.9.4: past 1 s).
pub const SHUTTERS: [Shutter; 45] = [
    shutter("1/2000", 500),
    shutter("1/1600", 625),
    shutter("1/1250", 800),
    shutter("1/1000", 1_000),
    shutter("1/800", 1_250),
    shutter("1/640", 1_563),
    shutter("1/500", 2_000),
    shutter("1/400", 2_500),
    shutter("1/320", 3_125),
    shutter("1/250", 4_000),
    shutter("1/200", 5_000),
    shutter("1/160", 6_250),
    shutter("1/125", 8_000),
    shutter("1/100", 10_000),
    shutter("1/80", 12_500),
    shutter("1/60", 16_667),
    shutter("1/50", 20_000),
    shutter("1/40", 25_000),
    shutter("1/30", 33_333),
    shutter("1/25", 40_000),
    shutter("1/20", 50_000),
    shutter("1/15", 66_667),
    shutter("1/13", 76_923),
    shutter("1/10", 100_000),
    shutter("1/8", 125_000),
    shutter("1/6", 166_667),
    shutter("1/5", 200_000),
    shutter("1/4", 250_000),
    shutter("0.3s", 300_000),
    shutter("0.4s", 400_000),
    shutter("0.5s", 500_000),
    shutter("0.6s", 600_000),
    shutter("0.8s", 800_000),
    shutter("1s", 1_000_000),
    shutter("1.3s", 1_300_000),
    shutter("1.6s", 1_600_000),
    shutter("2s", 2_000_000),
    shutter("2.5s", 2_500_000),
    shutter("3.2s", 3_200_000),
    shutter("4s", 4_000_000),
    shutter("5s", 5_000_000),
    shutter("6s", 6_000_000),
    shutter("8s", 8_000_000),
    shutter("10s", 10_000_000),
    shutter("11.7s", LONGEST_US),
];

/// The ISO dial: 100 to 800 in thirds. ISO 100 is the sensor's analogue
/// gain of 1.
pub const ISOS: [u32; 10] = [100, 125, 160, 200, 250, 320, 400, 500, 640, 800];

/// The analogue gain for an ISO.
pub fn iso_gain(iso: u32) -> f32 {
    iso as f32 / 100.0
}

/// The most analogue gain asked of the IMX219 (its own limit is 10.67; the
/// tuning's exposure profiles stop at 10).
pub const MAX_GAIN: f32 = 10.0;

/// Exposure compensation: thirds of a stop either way.
pub const EV_THIRDS: i8 = 9;

/// The preview's longest frame, and with night on: 1/15 s and 1/5 s.
pub const PREVIEW_CAP_US: u32 = 66_667;
pub const NIGHT_CAP_US: u32 = 200_000;

/// A shutter slower than this, hand-held, risks a blurred photo (1/30 s,
/// about one over the lens's 29 mm, as photographers reckon it).
pub const STEADY_US: u32 = 34_000;

/// The lens: fixed focus, fixed aperture.
pub const APERTURE: &str = "f/2.0";

/// White balance presets, by the colour temperature they fix.
pub const WB_PRESETS: [(&str, u32); 5] = [("daylight", 5500), ("cloudy", 6500), ("shade", 7500), ("tungsten", 3200), ("fluorescent", 4000)];
/// The colour temperatures the white balance dial runs between.
pub const KELVIN_MIN: u32 = 2500;
pub const KELVIN_MAX: u32 = 8000;
pub const KELVIN_STEP: u32 = 100;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Wb {
    #[default]
    Auto,
    Kelvin(u32),
    /// From a grey card (camkit.rs): the red and blue gains, in thousandths.
    Grey(u16, u16),
}

impl Wb {
    /// How the dial shows it: "awb", a preset's name, "5200K", or "grey".
    pub fn words(self) -> String {
        match self {
            Wb::Auto => "awb".into(),
            Wb::Kelvin(k) => format!("{k}K"),
            Wb::Grey(..) => "grey".into(),
        }
    }

    /// A grey card's gains (red, blue).
    pub fn gains(self) -> Option<[f32; 2]> {
        match self {
            Wb::Grey(r, b) => Some([f32::from(r) / 1000.0, f32::from(b) / 1000.0]),
            _ => None,
        }
    }

    /// The white balance a grey card's gains give.
    pub fn grey(gains: [f32; 2]) -> Wb {
        let milli = |g: f32| (g * 1000.0).round().clamp(100.0, 9000.0) as u16;
        Wb::Grey(milli(gains[0]), milli(gains[1]))
    }

    /// A preset's name, if the temperature is one.
    pub fn preset(self) -> Option<&'static str> {
        match self {
            Wb::Kelvin(k) => WB_PRESETS.iter().find(|(_, pk)| *pk == k).map(|(n, _)| *n),
            Wb::Auto | Wb::Grey(..) => None,
        }
    }

    /// The next preset, as the white balance key cycles them: auto, then
    /// each preset, then auto again.
    pub fn next_preset(self) -> Wb {
        match self.preset().and_then(|p| WB_PRESETS.iter().position(|(n, _)| *n == p)) {
            Some(i) if i + 1 < WB_PRESETS.len() => Wb::Kelvin(WB_PRESETS[i + 1].1),
            Some(_) => Wb::Auto,
            None if self == Wb::Auto => Wb::Kelvin(WB_PRESETS[0].1),
            None => Wb::Auto,
        }
    }
}

/// The pro dials as set: `None` is auto.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Pro {
    /// An index into [`SHUTTERS`].
    pub shutter: Option<usize>,
    /// An index into [`ISOS`].
    pub iso: Option<usize>,
    /// Thirds of a stop.
    pub ev: i8,
    pub wb: Wb,
}

impl Pro {
    /// Both shutter and ISO by hand: the exposure is wholly the user's.
    pub fn manual(&self) -> bool {
        self.shutter.is_some() && self.iso.is_some()
    }

    pub fn ev_words(&self) -> String {
        ev_words(self.ev)
    }

    /// Kept as a line in the Camera's own settings file.
    pub fn to_line(&self) -> String {
        let opt = |o: Option<usize>| o.map_or("auto".to_owned(), |i| i.to_string());
        let wb = match self.wb {
            Wb::Auto => "auto".to_owned(),
            Wb::Kelvin(k) => k.to_string(),
            Wb::Grey(r, b) => format!("g{r}:{b}"),
        };
        format!("shutter={} iso={} ev={} wb={wb}", opt(self.shutter), opt(self.iso), self.ev)
    }

    pub fn from_line(line: &str) -> Pro {
        let mut pro = Pro::default();
        for word in line.split_whitespace() {
            let Some((k, v)) = word.split_once('=') else { continue };
            match k {
                "shutter" => pro.shutter = v.parse().ok().filter(|i| *i < SHUTTERS.len()),
                "iso" => pro.iso = v.parse().ok().filter(|i| *i < ISOS.len()),
                "ev" => pro.ev = v.parse::<i8>().unwrap_or(0).clamp(-EV_THIRDS, EV_THIRDS),
                "wb" => {
                    let grey = v.strip_prefix('g').and_then(|g| g.split_once(':')).and_then(|(r, b)| Some(Wb::Grey(r.parse().ok()?, b.parse().ok()?)));
                    pro.wb = grey.unwrap_or_else(|| v.parse::<u32>().ok().filter(|k| (KELVIN_MIN..=KELVIN_MAX).contains(k)).map_or(Wb::Auto, Wb::Kelvin));
                }
                _ => {}
            }
        }
        pro
    }
}

/// Thirds of a stop as written: "±0", "+1/3", "−1 2/3", "+2".
pub fn ev_words(thirds: i8) -> String {
    if thirds == 0 {
        return "±0".into();
    }
    let sign = if thirds > 0 { "+" } else { "−" };
    let (whole, part) = (thirds.unsigned_abs() / 3, thirds.unsigned_abs() % 3);
    match (whole, part) {
        (w, 0) => format!("{sign}{w}"),
        (0, p) => format!("{sign}{p}/3"),
        (w, p) => format!("{sign}{w} {p}/3"),
    }
}

// --------------------------------------------------------------- metering ---

/// What the camera said of a frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Metered {
    pub exposure_us: u32,
    pub analogue: f32,
    pub digital: f32,
    /// White balance's red and blue gains.
    pub gains: [f32; 2],
    pub ct: u32,
    pub lux: f32,
    /// The exposure has settled (or is held by hand).
    pub settled: bool,
    /// Focus figure of merit, from the ISP's statistics (larger is sharper).
    pub fom: u32,
    /// The preview's scale against the photo's pixels (1 in the 1:1 check
    /// on the full sensor, 0.5 when only the binned mode could be cropped).
    pub zoom: f32,
}

impl Default for Metered {
    fn default() -> Self {
        Metered { exposure_us: 10_000, analogue: 1.0, digital: 1.0, gains: [1.8, 1.5], ct: 5000, lux: 400.0, settled: false, fom: 0, zoom: 0.0 }
    }
}

impl Metered {
    /// The camera program's line: `exp=16000 ag=2.000 dg=1.000 r=1.90
    /// b=1.60 ct=4800 lux=210 ae=1 fom=1234 zoom=0`; what is missing keeps
    /// its usual value.
    pub fn parse(line: &str) -> Option<Metered> {
        let mut m = Metered::default();
        let mut any = false;
        for word in line.split_whitespace() {
            let Some((k, v)) = word.split_once('=') else { continue };
            let f = v.parse::<f32>().ok().filter(|f| f.is_finite());
            let Some(f) = f else { continue };
            any |= matches!(k, "exp" | "ag" | "dg" | "r" | "b" | "ct" | "lux" | "ae" | "fom" | "zoom");
            match k {
                "exp" => m.exposure_us = f.max(1.0) as u32,
                "ag" => m.analogue = f.max(1.0),
                "dg" => m.digital = f.max(0.1),
                "r" => m.gains[0] = f.max(0.1),
                "b" => m.gains[1] = f.max(0.1),
                "ct" => m.ct = f.max(0.0) as u32,
                "lux" => m.lux = f.max(0.0),
                "ae" => m.settled = f >= 0.5,
                "fom" => m.fom = f.max(0.0) as u32,
                "zoom" => m.zoom = f.max(0.0),
                _ => {}
            }
        }
        any.then_some(m)
    }

    /// The whole exposure: time × gain (microseconds at a gain of 1).
    pub fn total(&self) -> f32 {
        self.exposure_us as f32 * self.analogue * self.digital
    }

    /// The ISO the gain amounts to.
    pub fn iso(&self) -> u32 {
        (100.0 * self.analogue * self.digital).round() as u32
    }
}

/// A shutter time as a camera writes it: the nearest mark on the dial
/// (within a sixth of a stop), else 1/N or seconds.
pub fn shutter_words(us: u32) -> String {
    let us = us.max(1);
    let near = SHUTTERS.iter().min_by(|a, b| stops_apart(a.us, us).total_cmp(&stops_apart(b.us, us)));
    if let Some(s) = near.filter(|s| stops_apart(s.us, us) <= 1.0 / 6.0) {
        return s.label.to_owned();
    }
    if us >= 300_000 {
        let s = us as f32 / 1e6;
        if s >= 10.0 { format!("{s:.0}s") } else { format!("{s:.1}s") }
    } else {
        format!("1/{}", (1e6 / us as f32).round() as u32)
    }
}

/// An ISO as a camera writes it: the nearest third of a stop.
pub fn iso_words(iso: u32) -> String {
    const MARKS: [u32; 19] = [100, 125, 160, 200, 250, 320, 400, 500, 640, 800, 1000, 1250, 1600, 2000, 2500, 3200, 4000, 5000, 6400];
    let iso = iso.max(1);
    let near = MARKS.iter().copied().min_by(|a, b| stops_apart(*a, iso).total_cmp(&stops_apart(*b, iso))).unwrap_or(100);
    format!("ISO {near}")
}

fn stops_apart(a: u32, b: u32) -> f32 {
    (a.max(1) as f32 / b.max(1) as f32).log2().abs()
}

/// The index on the shutter dial nearest a time (where a dial set by hand
/// starts from the exposure the camera had chosen).
pub fn nearest_shutter(us: u32) -> usize {
    (0..SHUTTERS.len()).min_by(|a, b| stops_apart(SHUTTERS[*a].us, us).total_cmp(&stops_apart(SHUTTERS[*b].us, us))).unwrap_or(12)
}

/// The index on the ISO dial nearest a gain.
pub fn nearest_iso(gain: f32) -> usize {
    let iso = (gain.max(1.0) * 100.0) as u32;
    (0..ISOS.len()).min_by(|a, b| stops_apart(ISOS[*a], iso).total_cmp(&stops_apart(ISOS[*b], iso))).unwrap_or(0)
}

/// The exposure a mid-tone needs, times the light (µs at a gain of 1, in
/// lux): the IMX219 tuning's reference, 27.7 ms at gain 1 under 998 lux.
pub const LUX_REFERENCE: f32 = 27_685.0 * 998.0;

/// Fully manual, the meter: how far the exposure set is from what the light
/// needs (stops, + brighter), by the camera's own measure of the light (its
/// Lux, worked out every frame whatever the exposure).
pub fn meter(m: &Metered, exposure_us: u32, gain: f32) -> f32 {
    (exposure_us as f32 * gain / (LUX_REFERENCE / m.lux.max(0.01))).log2()
}

/// Whether a shutter time is too long to hold steady by hand.
pub fn shaky(us: u32) -> bool {
    us > STEADY_US
}

// --------------------------------------------------------------- controls ---

/// What the preview runs with: fixed values where the dials are set (`None`
/// leaves it to the camera's own exposure), the compensation, a white
/// balance temperature (or auto), and its longest frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PreviewControls {
    pub exposure_us: Option<u32>,
    pub gain: Option<f32>,
    pub ev: f32,
    pub ct: Option<u32>,
    pub frame_max_us: u32,
    /// A grey card's white balance gains (red, blue), before `ct`.
    pub gains: Option<[f32; 2]>,
}

impl PreviewControls {
    /// The order to the camera program: `pro exp=8000 gain=auto ev=0.33
    /// ct=auto max=66667`.
    pub fn order(&self) -> String {
        let exp = self.exposure_us.map_or("auto".to_owned(), |e| e.to_string());
        let gain = self.gain.map_or("auto".to_owned(), |g| format!("{g:.3}"));
        let ct = self.ct.map_or("auto".to_owned(), |k| k.to_string());
        let gains = self.gains.map_or(String::new(), |[r, b]| format!(" gains={r:.4},{b:.4}"));
        format!("pro exp={exp} gain={gain} ev={:.3} ct={ct} max={}{gains}", self.ev, self.frame_max_us)
    }
}

/// The preview's controls for the dials (`None`: the plain photo mode).
pub fn preview(pro: Option<&Pro>, night: bool) -> PreviewControls {
    let cap = if night { NIGHT_CAP_US } else { PREVIEW_CAP_US };
    let Some(pro) = pro else { return PreviewControls { exposure_us: None, gain: None, ev: 0.0, ct: None, frame_max_us: cap, gains: None } };
    let mut exposure_us = pro.shutter.map(|i| SHUTTERS[i].us);
    let mut gain = pro.iso.map(|i| iso_gain(ISOS[i]));
    // Longer than a frame: shown at a frame's length, the gain making up the
    // rest, so the viewfinder keeps its pace and its brightness tells the truth.
    if let Some(e) = exposure_us.filter(|e| *e > cap) {
        exposure_us = Some(cap);
        if let Some(g) = gain {
            gain = Some((g * e as f32 / cap as f32).min(MAX_GAIN));
        }
    }
    let ev = if pro.manual() { 0.0 } else { f32::from(pro.ev) / 3.0 };
    let ct = match pro.wb {
        Wb::Kelvin(k) => Some(k),
        Wb::Auto | Wb::Grey(..) => None,
    };
    PreviewControls { exposure_us, gain, ev, ct, frame_max_us: cap.max(exposure_us.unwrap_or(0)), gains: pro.wb.gains() }
}

/// The sensor's noise filter for a photo, as libcamera's NoiseReductionMode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Denoise {
    Off,
    Fast,
    HighQuality,
}

impl Denoise {
    pub fn word(self) -> &'static str {
        match self {
            Denoise::Off => "off",
            Denoise::Fast => "fast",
            Denoise::HighQuality => "hq",
        }
    }
}

/// What a photo is taken with: all of it fixed, so the full sensor's first
/// frame is the photo.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StillControls {
    pub exposure_us: u32,
    pub gain: f32,
    /// The preview's white balance gains (red, blue), or a temperature.
    pub gains: Option<[f32; 2]>,
    pub ct: Option<u32>,
    pub sharpness: f32,
    pub denoise: Denoise,
}

impl StillControls {
    /// Its words in a still order: `exp=… gain=… gains=r,b sharp=… nr=…`,
    /// tab between each.
    pub fn words(&self) -> Vec<String> {
        let mut out = vec![format!("exp={}", self.exposure_us), format!("gain={:.3}", self.gain)];
        if let Some([r, b]) = self.gains {
            out.push(format!("gains={r:.4},{b:.4}"));
        }
        if let Some(k) = self.ct {
            out.push(format!("ct={k}"));
        }
        out.push(format!("sharp={:.2}", self.sharpness));
        out.push(format!("nr={}", self.denoise.word()));
        out
    }
}

/// The hand-held split: shutter and gain, step by step, as the light falls.
/// Each step's whole exposure is the last one's or more, so any exposure
/// lands on exactly one place along it.
fn steps(longest_us: u32) -> [(f32, f32); 10] {
    [
        (100.0, 1.0),
        (10_000.0, 1.0),
        (10_000.0, 2.0),
        (16_667.0, 2.0),
        (16_667.0, 4.0),
        (25_000.0, 4.0),
        (25_000.0, 8.0),
        (33_333.0, 8.0),
        (33_333.0, MAX_GAIN),
        (longest_us.max(33_333) as f32, MAX_GAIN),
    ]
}

/// A whole exposure (µs at a gain of 1) split for a hand-held photo, the
/// shutter no longer than `longest_us`.
pub fn split(total: f32, longest_us: u32) -> (u32, f32) {
    let steps = steps(longest_us);
    let total = total.max(steps[0].0);
    for pair in steps.windows(2) {
        let ((e0, g0), (e1, g1)) = (pair[0], pair[1]);
        if total <= e1 * g1 {
            return if e1 > e0 { ((total / g0).round() as u32, g0) } else { (e0 as u32, (total / e0).clamp(g0, g1)) };
        }
    }
    // More than it can give: the longest shutter at the most gain.
    let (e, g) = steps[steps.len() - 1];
    (e as u32, g)
}

/// A photo's noise filter and sharpening for the gain it takes: little gain,
/// little noise, so no filter to smear the detail and a firmer sharpening;
/// more gain, the filter and the usual sharpening.
pub fn processing(gain: f32) -> (f32, Denoise) {
    match gain {
        g if g < 1.6 => (1.5, Denoise::Off),
        g if g < 3.0 => (1.3, Denoise::Fast),
        g if g < 6.0 => (1.1, Denoise::HighQuality),
        _ => (1.0, Denoise::HighQuality),
    }
}

/// What a photo is taken with: the dials where set, else the exposure the
/// preview settled on, split for a steady hand.
pub fn still(pro: Option<&Pro>, m: &Metered, night: bool) -> StillControls {
    let longest = if night { NIGHT_CAP_US } else { PREVIEW_CAP_US };
    let total = m.total();
    let (exposure_us, gain) = match pro.map(|p| (p.shutter.map(|i| SHUTTERS[i].us), p.iso.map(|i| iso_gain(ISOS[i])))) {
        // Both by hand: as set.
        Some((Some(e), Some(g))) => (e, g),
        // Shutter priority: the gain that keeps the metered exposure.
        Some((Some(e), None)) => (e, (total / e as f32).clamp(1.0, MAX_GAIN)),
        // ISO priority: the time that keeps it, up to a quarter of a second.
        Some((None, Some(g))) => ((total / g).round().clamp(100.0, 250_000.0) as u32, g),
        _ => split(total, longest),
    };
    let ct = pro.and_then(|p| match p.wb {
        Wb::Kelvin(k) => Some(k),
        Wb::Auto | Wb::Grey(..) => None,
    });
    let grey = pro.and_then(|p| p.wb.gains());
    let (sharpness, denoise) = processing(gain);
    StillControls { exposure_us, gain, gains: if ct.is_some() { None } else { Some(grey.unwrap_or(m.gains)) }, ct, sharpness, denoise }
}

/// Below this, detail mode's "auto" merges a burst (lux).
pub const DETAIL_LUX: f32 = 25.0;

/// Real low light, where detail mode's "auto" merges a burst: under 25 lux
/// by the camera's own measure, and the photo out of room as well (all of
/// the gain at a shutter of 1/20 s or slower), both. 0.9.3 took either,
/// and a gain of 8 at 1/40 s, which an evening room under about 140 lux
/// needs: every photo indoors at night was a burst, its "hold still" for
/// all of it.
pub fn low_light(m: &Metered, s: &StillControls) -> bool {
    m.lux < DETAIL_LUX && s.gain >= MAX_GAIN - 0.01 && s.exposure_us >= 50_000
}

// ------------------------------------------------- histogram and zebras ---

/// The histogram's bins.
pub const BINS: usize = 64;

/// Zebras mark a pixel with any channel at this or over (98%).
pub const ZEBRA_AT: u8 = 250;

/// Whether a pixel is clipping, as the zebras mark it.
#[cfg(test)]
pub fn clipping(r: u8, g: u8, b: u8) -> bool {
    r.max(g).max(b) >= ZEBRA_AT
}

/// Whether a clipping pixel at (x, y) is on a zebra stripe: diagonal
/// stripes four pixels wide, marching with `phase`.
pub fn zebra_stripe(x: usize, y: usize, phase: usize) -> bool {
    ((x + y + phase) / 4) % 2 == 0
}

/// A picture's brightness in [`BINS`] bins, and the share of it clipping.
#[derive(Clone, Debug, PartialEq)]
pub struct Histogram {
    pub bins: [u32; BINS],
    pub clipped: f32,
}

impl Default for Histogram {
    fn default() -> Self {
        Histogram { bins: [0; BINS], clipped: 0.0 }
    }
}

impl Histogram {
    /// Of RGB pixels (BT.601 brightness).
    #[cfg(test)]
    pub fn of(rgb: &[u8]) -> Histogram {
        let mut h = Histogram::default();
        let mut clipped = 0u32;
        let mut n = 0u32;
        for p in rgb.chunks_exact(3) {
            h.add(p[0], p[1], p[2]);
            clipped += u32::from(clipping(p[0], p[1], p[2]));
            n += 1;
        }
        h.clipped = if n > 0 { clipped as f32 / n as f32 } else { 0.0 };
        h
    }

    /// Of a frame's brightness plane, every `step`th pixel each way (the
    /// camera's own Y: what it measured, before any look). Its clipping
    /// share is of brightness alone; the zebras' count is the closer one.
    pub fn of_luma(y: &[u8], w: usize, h: usize, step: usize) -> Histogram {
        let mut out = Histogram::default();
        let step = step.max(1);
        let (mut n, mut clipped) = (0u32, 0u32);
        for row in (0..h).step_by(step) {
            for v in y[row * w..(row + 1) * w].iter().step_by(step) {
                out.bins[(usize::from(*v) * BINS) >> 8] += 1;
                clipped += u32::from(*v >= ZEBRA_AT);
                n += 1;
            }
        }
        out.clipped = if n > 0 { clipped as f32 / n as f32 } else { 0.0 };
        out
    }

    #[cfg(test)]
    pub fn add(&mut self, r: u8, g: u8, b: u8) {
        let y = (77 * u32::from(r) + 150 * u32::from(g) + 29 * u32::from(b)) >> 8;
        self.bins[(y as usize * BINS) >> 8] += 1;
    }

    /// Each bin's height, 0 to 1 of the tallest (square-rooted, as cameras
    /// draw it, so a few dark pixels still show).
    pub fn heights(&self) -> [f32; BINS] {
        let top = self.bins.iter().copied().max().unwrap_or(0).max(1) as f32;
        let mut out = [0.0; BINS];
        for (o, b) in out.iter_mut().zip(self.bins) {
            *o = (b as f32 / top).sqrt();
        }
        out
    }
}

// ---------------------------------------------------------- focus peaking ---

/// Edges strong enough to be in focus, from a picture's brightness: each
/// pixel's gradient against the strongest few percent of the frame, never
/// less than `PEAK_FLOOR` (so a flat, blurred frame shows none).
pub const PEAK_FLOOR: u16 = 36;

/// The pixels to mark: true where the brightness's gradient is among the
/// strongest of the frame.
pub fn peaks(y: &[u8], w: usize, h: usize) -> Vec<bool> {
    let mut out = vec![false; w * h];
    if w < 3 || h < 3 || y.len() < w * h {
        return out;
    }
    let mut grads = vec![0u16; w * h];
    let mut counts = [0u32; 512];
    for row in 1..h - 1 {
        for x in 1..w - 1 {
            let at = row * w + x;
            let gx = (i32::from(y[at + 1]) - i32::from(y[at - 1])).unsigned_abs();
            let gy = (i32::from(y[at + w]) - i32::from(y[at - w])).unsigned_abs();
            let g = (gx + gy).min(511) as u16;
            grads[at] = g;
            counts[g as usize] += 1;
        }
    }
    // The strongest 6% of the frame, or the floor.
    let want = ((w - 2) * (h - 2)) as u32 * 6 / 100;
    let (mut seen, mut threshold) = (0u32, 511u16);
    for g in (0..512).rev() {
        seen += counts[g];
        if seen >= want {
            threshold = g as u16;
            break;
        }
    }
    let threshold = threshold.max(PEAK_FLOOR);
    for (o, g) in out.iter_mut().zip(&grads) {
        *o = *g >= threshold;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shutter_and_iso_turn_into_the_controls_libcamera_takes() {
        // The dial's marks in microseconds: 1/125 is 8 ms, 1 s a million.
        assert_eq!(SHUTTERS[12].label, "1/125");
        assert_eq!(SHUTTERS[12].us, 8_000);
        assert_eq!(SHUTTERS.first().map(|s| s.us), Some(500));
        assert_eq!(SHUTTERS.last().map(|s| s.us), Some(LONGEST_US));
        assert!(SHUTTERS.windows(2).all(|w| w[1].us > w[0].us), "longer each step");
        // Each step a third of a stop (within the marks' rounding).
        for w in SHUTTERS.windows(2) {
            let stops = (w[1].us as f32 / w[0].us as f32).log2();
            // (The marks are rounded: 1/15 to 1/13 is the widest apart.)
            assert!((stops - 1.0 / 3.0).abs() < 0.22, "{} → {}: {stops}", w[0].label, w[1].label);
        }
        // ISO is analogue gain: 100 is 1, 400 is 4, 800 is 8.
        assert_eq!(iso_gain(100), 1.0);
        assert_eq!(iso_gain(400), 4.0);
        assert_eq!(iso_gain(ISOS[ISOS.len() - 1]), 8.0);

        // Both set by hand, short enough for a frame: as set, and no
        // compensation (there is nothing for it to move).
        let pro = Pro { shutter: Some(12), iso: Some(6), ev: 3, wb: Wb::Kelvin(5500) };
        let p = preview(Some(&pro), false);
        assert_eq!(p.exposure_us, Some(8_000));
        assert_eq!(p.gain, Some(4.0));
        assert_eq!(p.ev, 0.0);
        assert_eq!(p.ct, Some(5500));
        assert_eq!(p.order(), "pro exp=8000 gain=4.000 ev=0.000 ct=5500 max=66667");
        let s = still(Some(&pro), &Metered::default(), false);
        assert_eq!((s.exposure_us, s.gain, s.ct, s.gains), (8_000, 4.0, Some(5500), None));

        // A second at ISO 100: the preview at a frame's length with fifteen
        // times the gain (the same brightness), the photo the whole second.
        let long = Pro { shutter: Some(nearest_shutter(1_000_000)), iso: Some(0), ev: 0, wb: Wb::Auto };
        let p = preview(Some(&long), false);
        assert_eq!(p.exposure_us, Some(PREVIEW_CAP_US));
        assert!((p.gain.unwrap() - MAX_GAIN).abs() < 1e-3, "capped at the sensor's gain: {:?}", p.gain);
        let half = Pro { shutter: Some(nearest_shutter(250_000)), iso: Some(0), ev: 0, wb: Wb::Auto };
        let p = preview(Some(&half), false);
        assert!((p.gain.unwrap() - 250_000.0 / PREVIEW_CAP_US as f32).abs() < 0.01, "{:?}", p.gain);
        assert_eq!(still(Some(&long), &Metered::default(), false).exposure_us, 1_000_000);

        // Shutter priority: the gain keeps what the preview metered.
        let m = Metered { exposure_us: 20_000, analogue: 2.0, digital: 1.0, ..Metered::default() };
        let sp = Pro { shutter: Some(nearest_shutter(10_000)), ..Pro::default() };
        let s = still(Some(&sp), &m, false);
        assert_eq!(s.exposure_us, 10_000);
        assert!((s.gain - 4.0).abs() < 1e-3);
        assert_eq!(preview(Some(&sp), false).gain, None, "the camera picks the gain");
        // ISO priority: the time keeps it.
        let ip = Pro { iso: Some(nearest_iso(8.0)), ..Pro::default() };
        let s = still(Some(&ip), &m, false);
        assert_eq!((s.exposure_us, s.gain), (5_000, 8.0));
        // Compensation in thirds, and white balance by temperature.
        assert_eq!(preview(Some(&Pro { ev: -4, ..Pro::default() }), false).ev, -4.0 / 3.0);
        assert_eq!(ev_words(-4), "−1 1/3");
        assert_eq!(ev_words(3), "+1");
        assert_eq!(ev_words(2), "+2/3");
        assert_eq!(ev_words(0), "±0");
    }

    #[test]
    fn auto_photos_keep_the_shutter_short_and_the_preview_settled() {
        // Bright: all shutter at ISO 100.
        assert_eq!(split(4_000.0, PREVIEW_CAP_US), (4_000, 1.0));
        // A room (about 300 lux): no slower than 1/60 while gain is left.
        let (e, g) = split(60_000.0, PREVIEW_CAP_US);
        assert_eq!(e, 16_667);
        assert!((g - 3.6).abs() < 0.01, "{g}");
        // Dim: 1/40 at 8 before 1/30.
        let (e, g) = split(200_000.0, PREVIEW_CAP_US);
        assert_eq!(e, 25_000);
        assert!((g - 8.0).abs() < 1e-3);
        // Darker than it can give: the longest at the most gain.
        assert_eq!(split(1e9, PREVIEW_CAP_US), (PREVIEW_CAP_US, MAX_GAIN));
        assert_eq!(split(1e9, NIGHT_CAP_US), (NIGHT_CAP_US, MAX_GAIN));
        // Every exposure within reach is kept exactly, and the shutter never
        // gets shorter as the light falls.
        let mut last = 0;
        for i in 0..200 {
            let total = 150.0 * 1.07f32.powi(i);
            let (e, g) = split(total, PREVIEW_CAP_US);
            assert!(e >= last, "{total}: {e} after {last}");
            last = e;
            if total < PREVIEW_CAP_US as f32 * MAX_GAIN {
                assert!(((e as f32 * g) / total - 1.0).abs() < 0.01, "{total}: {e} × {g}");
            }
        }
        // The photo carries the preview's white balance gains as they were.
        let m = Metered { exposure_us: 30_000, analogue: 2.0, gains: [1.92, 1.51], ..Metered::default() };
        let s = still(None, &m, false);
        assert_eq!(s.gains, Some([1.92, 1.51]));
        assert_eq!(s.ct, None);
        assert_eq!(s.exposure_us, 16_667, "shorter than the preview's 1/33");
        assert!(s.words().iter().any(|w| w == "gains=1.9200,1.5100"));
        // Little gain: no noise filter and firmer sharpening; much: the filter.
        assert_eq!(processing(1.0), (1.5, Denoise::Off));
        assert_eq!(processing(8.0).1, Denoise::HighQuality);
        // Detail's auto only in real low light.
        assert!(!low_light(&m, &s));
        let dark = Metered { exposure_us: 66_667, analogue: 8.0, lux: 18.0, ..Metered::default() };
        assert!(low_light(&dark, &still(None, &dark, false)));
    }

    #[test]
    fn metering_lines_and_words_read_as_a_camera_writes_them() {
        let m = Metered::parse("exp=16000 ag=2.000 dg=1.250 r=1.90 b=1.60 ct=4800 lux=210 ae=1 fom=1234 zoom=1").unwrap();
        assert_eq!(m.exposure_us, 16_000);
        assert!((m.total() - 40_000.0).abs() < 1.0);
        assert_eq!(m.iso(), 250);
        assert!(m.settled && m.zoom == 1.0 && m.fom == 1234 && m.ct == 4800);
        assert!(Metered::parse("nothing here").is_none());
        assert_eq!(shutter_words(8_000), "1/125");
        assert_eq!(shutter_words(16_000), "1/60");
        assert_eq!(shutter_words(1_000_000), "1s");
        assert_eq!(shutter_words(3_000_000), "3.2s");
        assert_eq!(shutter_words(14_000_000), "14s");
        assert_eq!(iso_words(242), "ISO 250");
        assert!(shaky(66_667) && !shaky(33_333) && !shaky(16_667));
        // The meter: right at the tuning's reference, a stop over at twice.
        let lit = Metered { lux: 998.0, ..Metered::default() };
        assert!(meter(&lit, 27_685, 1.0).abs() < 1e-3);
        assert!((meter(&lit, 27_685, 2.0) - 1.0).abs() < 1e-3);
        // Pro's dials kept and read back.
        let pro = Pro { shutter: Some(5), iso: None, ev: -2, wb: Wb::Kelvin(3200) };
        assert_eq!(Pro::from_line(&pro.to_line()), pro);
        assert_eq!(Wb::Kelvin(3200).preset(), Some("tungsten"));
        assert_eq!(Wb::Auto.next_preset(), Wb::Kelvin(5500));
        assert_eq!(Wb::Kelvin(4000).next_preset(), Wb::Auto);
    }

    #[test]
    fn the_histogram_counts_brightness_and_the_zebras_mark_from_98_percent() {
        let mut rgb = Vec::new();
        for _ in 0..90 {
            rgb.extend_from_slice(&[0, 0, 0]);
        }
        for _ in 0..10 {
            rgb.extend_from_slice(&[255, 255, 255]);
        }
        let h = Histogram::of(&rgb);
        assert_eq!(h.bins[0], 90);
        assert_eq!(h.bins[BINS - 1], 10);
        assert_eq!(h.bins.iter().sum::<u32>(), 100);
        assert!((h.clipped - 0.1).abs() < 1e-6);
        let heights = h.heights();
        assert_eq!(heights[0], 1.0);
        assert!((heights[BINS - 1] - (10.0f32 / 90.0).sqrt()).abs() < 1e-6);
        // Mid grey lands in the middle.
        let mid = Histogram::of(&[128, 128, 128]);
        assert_eq!(mid.bins[32], 1);
        // Zebras from 250 in any channel, not 249.
        assert!(!clipping(249, 249, 249));
        assert!(clipping(250, 0, 0) && clipping(0, 0, 255));
        assert!(!clipping(200, 240, 249));
        // Stripes four wide on the diagonal, marching with the phase.
        assert!(zebra_stripe(0, 0, 0) && zebra_stripe(3, 0, 0) && !zebra_stripe(4, 0, 0) && !zebra_stripe(0, 4, 0));
        assert!(!zebra_stripe(0, 0, 4) && zebra_stripe(0, 0, 8));
    }

    #[test]
    fn focus_peaking_marks_hard_edges_and_not_a_blur() {
        let (w, h) = (64, 48);
        // A sharp step down the middle.
        let sharp: Vec<u8> = (0..w * h).map(|i| if i % w < w / 2 { 30 } else { 220 }).collect();
        let marks = peaks(&sharp, w, h);
        assert!(marks[10 * w + w / 2] && marks[10 * w + w / 2 - 1], "the edge is marked");
        assert!(!marks[10 * w + 5] && !marks[10 * w + w - 5], "the flat sides are not");
        // The same step blurred over twenty pixels: nothing strong enough.
        let soft: Vec<u8> = (0..w * h).map(|i| (30.0 + 190.0 * (((i % w) as f32 - 22.0) / 20.0).clamp(0.0, 1.0)) as u8).collect();
        assert!(!peaks(&soft, w, h).iter().any(|m| *m), "a blur shows no peaks");
    }
}
