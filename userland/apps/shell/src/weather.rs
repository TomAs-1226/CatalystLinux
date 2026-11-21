//! Weather: the forecast where the handheld is, from Open-Meteo.
//!
//! Open-Meteo (open-meteo.com) gives forecasts from the national weather
//! services' models free for non-commercial use, with no key and no
//! account. So the firmware carries no key that could leak or be revoked,
//! nobody signs up for anything, and every handheld simply asks: the
//! place's latitude and longitude go out, and nothing about the handheld.
//! Its place-name search (the geocoding API) is the same.
//!
//! It is asked with curl over HTTPS, on a thread of its own, when the app
//! opens on a forecast older than half an hour and every half hour while it
//! stays open. The last forecast is kept beside the settings
//! (`weather.json`), so the app opens on it at once, and offline it is still
//! there, saying when it is from.
//!
//! The place is the time zone's city, found by name once and kept, until
//! one is chosen (`l`). Units are °F and mph in the United States' zones and
//! °C and km/h elsewhere, until switched (`u`).

use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::{Duration, Instant};

use bezel::{Align, Rect, TextStyle};
use chrono::{NaiveDate, NaiveDateTime, Timelike};
use detent::{role, Animated};
use serde_json::{json, Value};

use crate::gfx::{Gfx, ListState, GAP, H, PAD, W};
use crate::platform::Key;
use crate::robot::Robot;
use crate::shell::{Cx, Nav, Screen};
use crate::tools::{Field, Typed};

const FORECAST: &str = "https://api.open-meteo.com/v1/forecast";
const GEOCODING: &str = "https://geocoding-api.open-meteo.com/v1/search";
/// What is asked for: now, the hours, and the week, in the place's own time.
const FIELDS: &str = "current=temperature_2m,apparent_temperature,relative_humidity_2m,weather_code,wind_speed_10m,is_day\
&hourly=temperature_2m,apparent_temperature,relative_humidity_2m,weather_code,wind_speed_10m,precipitation_probability,is_day\
&daily=weather_code,temperature_2m_max,temperature_2m_min,precipitation_probability_max,sunrise,sunset\
&timezone=auto&forecast_days=7";
/// The forecast is asked for again this often while the app is open, and
/// when it opens on one older than this.
const REFRESH: i64 = 30 * 60;
/// After a fetch that failed, the next try waits this long.
const RETRY: Duration = Duration::from_secs(5 * 60);
/// The forecast's own reading of now is shown this long after it; then the
/// hour's forecast stands in for it.
const CURRENT_FOR: i64 = 90 * 60;
/// Hours along the strip, and how many are in view at once.
const HOURS: usize = 24;
const CELLS: usize = 6;
/// The icons along the strip and the week.
const STRIP_ICON: f32 = 15.0;
const DAY_ICON: f32 = 16.0;
/// Whose forecasts they are, as the Place page credits them.
const CREDIT: &str = "forecasts by open-meteo.com · cc by 4.0";
/// The simulator's forecast: Sydney on a November afternoon, as Open-Meteo gave it.
const DEMO: &str = include_str!("weather-demo.json");

/// The simulator shows the forecast kept with it rather than asking the
/// network, unless told to (`CATALYST_WEATHER_LIVE`).
fn demo() -> bool {
    !cfg!(target_arch = "aarch64") && std::env::var_os("CATALYST_WEATHER_LIVE").is_none()
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

// ----------------------------------------------------------------- units ---

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Units {
    Metric,
    Imperial,
}

/// The zones of the United States and its territories, which keep °F.
const FAHRENHEIT_ZONES: &[&str] = &[
    "America/New_York",
    "America/Detroit",
    "America/Chicago",
    "America/Menominee",
    "America/Denver",
    "America/Boise",
    "America/Phoenix",
    "America/Los_Angeles",
    "America/Anchorage",
    "America/Juneau",
    "America/Sitka",
    "America/Metlakatla",
    "America/Yakutat",
    "America/Nome",
    "America/Adak",
    "America/Puerto_Rico",
    "Pacific/Honolulu",
    "Pacific/Guam",
    "Pacific/Saipan",
    "Pacific/Pago_Pago",
];
const FAHRENHEIT_AREAS: &[&str] = &["America/Indiana/", "America/Kentucky/", "America/North_Dakota/", "US/"];

impl Units {
    /// The units a zone's own forecasts use.
    pub fn of_zone(zone: &str) -> Units {
        if FAHRENHEIT_ZONES.contains(&zone) || FAHRENHEIT_AREAS.iter().any(|area| zone.starts_with(area)) {
            Units::Imperial
        } else {
            Units::Metric
        }
    }

    fn name(self) -> &'static str {
        match self {
            Units::Metric => "metric",
            Units::Imperial => "imperial",
        }
    }

    fn from_name(name: &str) -> Option<Units> {
        [Units::Metric, Units::Imperial].into_iter().find(|u| u.name() == name)
    }

    fn other(self) -> Units {
        match self {
            Units::Metric => Units::Imperial,
            Units::Imperial => Units::Metric,
        }
    }

    /// Degrees in these units, from Celsius.
    pub fn degrees(self, celsius: f32) -> f32 {
        match self {
            Units::Metric => celsius,
            Units::Imperial => celsius * 9.0 / 5.0 + 32.0,
        }
    }

    /// A speed in these units, from km/h.
    pub fn speed(self, kmh: f32) -> f32 {
        match self {
            Units::Metric => kmh,
            Units::Imperial => kmh * 0.621_371,
        }
    }

    /// A temperature as shown: whole degrees, with a true minus.
    pub fn temp(self, celsius: f32) -> String {
        format!("{}°", signed(self.degrees(celsius).round() as i32))
    }

    pub fn wind(self, kmh: f32) -> String {
        format!("{} {}", self.speed(kmh).round() as i32, self.speed_unit())
    }

    fn speed_unit(self) -> &'static str {
        match self {
            Units::Metric => "km/h",
            Units::Imperial => "mph",
        }
    }

    /// The scale, as the keys' line and the island name it.
    fn scale(self) -> &'static str {
        match self {
            Units::Metric => "°C",
            Units::Imperial => "°F",
        }
    }
}

fn signed(v: i32) -> String {
    if v < 0 {
        format!("−{}", -v)
    } else {
        v.to_string()
    }
}

// ------------------------------------------------------------ conditions ---

/// What a WMO weather code (the ones Open-Meteo gives) says, in a few words;
/// the clear ones by day and by night.
pub fn condition(code: u8, day: bool) -> &'static str {
    match code {
        0 if day => "Sunny",
        0 => "Clear",
        1 if day => "Mostly sunny",
        1 => "Mostly clear",
        2 => "Partly cloudy",
        3 => "Overcast",
        45 => "Fog",
        48 => "Icy fog",
        51 => "Light drizzle",
        53 => "Drizzle",
        55 => "Heavy drizzle",
        56 | 57 => "Icy drizzle",
        61 => "Light rain",
        63 => "Rain",
        65 => "Heavy rain",
        66 | 67 => "Freezing rain",
        71 => "Light snow",
        73 => "Snow",
        75 => "Heavy snow",
        77 => "Snow grains",
        80 => "Light showers",
        81 => "Showers",
        82 => "Heavy showers",
        85 => "Snow showers",
        86 => "Snow squalls",
        95 => "Thunderstorm",
        96 | 99 => "Hailstorm",
        _ => "Unsettled",
    }
}

/// The same in a word, for Home's tile.
fn word(code: u8, day: bool) -> &'static str {
    match code {
        0 | 1 if day => "sunny",
        0 | 1 => "clear",
        2 | 3 => "cloudy",
        45 | 48 => "fog",
        51..=57 => "drizzle",
        61..=65 => "rain",
        66 | 67 => "sleet",
        71..=77 | 85 | 86 => "snow",
        80..=82 => "showers",
        95 => "storms",
        96 | 99 => "hail",
        _ => "unsettled",
    }
}

/// The Material Symbol for a code, by day or by night. Rain of every
/// strength is the one cloud with drops (Material's light and heavy rain are
/// bare streaks, which at strip size read as hatching, not weather); the
/// words say how much.
pub fn icon(code: u8, day: bool) -> &'static str {
    match code {
        0 | 1 if day => "sunny",
        0 | 1 => "clear_night",
        2 if day => "partly_cloudy_day",
        2 => "partly_cloudy_night",
        3 => "cloud",
        45 | 48 => "foggy",
        51 | 53 | 55 | 61 | 63 | 65 | 80..=82 => "rainy",
        56 | 57 | 66 | 67 => "weather_mix",
        71..=77 | 85 | 86 => "weather_snowy",
        95 | 96 | 99 => "thunderstorm",
        _ => "cloud",
    }
}

/// Every code Open-Meteo gives.
#[cfg(test)]
const CODES: [u8; 28] = [0, 1, 2, 3, 45, 48, 51, 53, 55, 56, 57, 61, 63, 65, 66, 67, 71, 73, 75, 77, 80, 81, 82, 85, 86, 95, 96, 99];

// ---------------------------------------------------------------- places ---

#[derive(Clone, Debug, PartialEq)]
pub struct Place {
    pub name: String,
    /// Its state or province, where the search gave one.
    pub region: String,
    /// Its country's two-letter code.
    pub country: String,
    pub lat: f64,
    pub lon: f64,
}

impl Place {
    fn to_json(&self) -> Value {
        json!({"name": self.name, "region": self.region, "country": self.country, "lat": self.lat, "lon": self.lon})
    }

    fn from_json(v: &Value) -> Option<Place> {
        let text = |key: &str| v.get(key).and_then(Value::as_str).unwrap_or_default().to_owned();
        let place = Place { name: text("name"), region: text("region"), country: text("country"), lat: v.get("lat")?.as_f64()?, lon: v.get("lon")?.as_f64()? };
        (!place.name.is_empty() && place.lat.abs() <= 90.0 && place.lon.abs() <= 180.0).then_some(place)
    }

    /// The same place: where it is, to ten metres or so.
    fn same(&self, other: &Place) -> bool {
        (self.lat - other.lat).abs() < 1e-4 && (self.lon - other.lon).abs() < 1e-4
    }

    /// As a list names it: "Sydney · New South Wales".
    fn line(&self) -> String {
        if self.region.is_empty() || self.region == self.name {
            self.name.clone()
        } else {
            format!("{} · {}", self.name, self.region)
        }
    }
}

/// A place the search found, and the time zone it is in.
#[derive(Clone, Debug, PartialEq)]
pub struct Found {
    pub place: Place,
    pub zone: String,
}

/// The places in a reply from the place-name search, most populous first
/// (as Open-Meteo orders them).
pub fn parse_found(v: &Value) -> Vec<Found> {
    let Some(results) = v.get("results").and_then(Value::as_array) else { return Vec::new() };
    results
        .iter()
        .filter_map(|r| {
            let text = |key: &str| r.get(key).and_then(Value::as_str).unwrap_or_default().to_owned();
            let place = Place { name: text("name"), region: text("admin1"), country: text("country_code").to_uppercase(), lat: r.get("latitude")?.as_f64()?, lon: r.get("longitude")?.as_f64()? };
            (!place.name.is_empty()).then(|| Found { place, zone: text("timezone") })
        })
        .collect()
}

/// Of the places a zone's city finds, the one in that zone (there are
/// Sydneys in Nova Scotia and Montana too), else the most populous.
fn in_zone(found: &[Found], zone: &str) -> Option<Place> {
    found.iter().find(|f| f.zone == zone).or(found.first()).map(|f| f.place.clone())
}

/// The city a time zone is known by, to look up: its listed name without
/// the region ("New York"), else the zone's own last part ("America/Boise"
/// is Boise; "Asia/Kolkata" is listed as India, so Kolkata). None for UTC
/// and the other zones of no place.
pub fn zone_city(zone: &str) -> Option<String> {
    let (area, _) = zone.split_once('/')?;
    let last = zone.rsplit('/').next().unwrap_or_default();
    if area == "Etc" || last.is_empty() {
        return None;
    }
    const BY_COUNTRY: [&str; 2] = ["Asia/Kolkata", "Asia/Shanghai"];
    match crate::device::ZONES.iter().find(|z| z.id == zone && !BY_COUNTRY.contains(&zone)) {
        Some(listed) => Some(listed.city.split(" · ").next().unwrap_or(listed.city).to_owned()),
        None => Some(last.replace('_', " ")),
    }
}

/// The simulator's own place, with its forecast.
fn demo_place() -> Place {
    Place { name: "Sydney".into(), region: "New South Wales".into(), country: "AU".into(), lat: -33.8688, lon: 151.2093 }
}

/// What the simulator's search finds: a few places, by the start of their name.
const DEMO_PLACES: [(&str, &str, &str, f64, f64, &str); 10] = [
    ("San Jose", "California", "US", 37.3394, -121.895, "America/Los_Angeles"),
    ("San Francisco", "California", "US", 37.7749, -122.4194, "America/Los_Angeles"),
    ("San Diego", "California", "US", 32.7157, -117.1647, "America/Los_Angeles"),
    ("Santiago", "Santiago Metropolitan", "CL", -33.4569, -70.6483, "America/Santiago"),
    ("Sydney", "New South Wales", "AU", -33.8688, 151.2093, "Australia/Sydney"),
    ("New York", "New York", "US", 40.7143, -74.006, "America/New_York"),
    ("Houston", "Texas", "US", 29.7633, -95.3633, "America/Chicago"),
    ("Toronto", "Ontario", "CA", 43.7001, -79.4163, "America/Toronto"),
    ("London", "England", "GB", 51.5085, -0.1257, "Europe/London"),
    ("Melbourne", "Victoria", "AU", -37.814, 144.9633, "Australia/Melbourne"),
];

fn demo_search(name: &str) -> Vec<Found> {
    let name = name.trim().to_lowercase();
    DEMO_PLACES
        .iter()
        .filter(|p| !name.is_empty() && p.0.to_lowercase().starts_with(&name))
        .map(|&(n, region, country, lat, lon, zone)| Found { place: Place { name: n.into(), region: region.into(), country: country.into(), lat, lon }, zone: zone.into() })
        .collect()
}

// ----------------------------------------------------------------- prefs ---

/// Where the forecast is for, and in what units (`weather-place.json`).
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Prefs {
    /// The place chosen, or the time zone's city as last found.
    pub place: Option<Place>,
    /// Whether the place was chosen; otherwise it follows the time zone.
    pub chosen: bool,
    /// The zone the place was found for, while it follows the zone.
    pub zone: Option<String>,
    /// The units switched to; none follows the zone.
    pub units: Option<Units>,
}

impl Prefs {
    fn file() -> PathBuf {
        crate::config_dir().join("weather-place.json")
    }

    pub fn load() -> Prefs {
        let Some(v) = std::fs::read_to_string(Self::file()).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()) else { return Prefs::default() };
        Prefs {
            place: v.get("place").and_then(Place::from_json),
            chosen: v.get("chosen").and_then(Value::as_bool).unwrap_or(false),
            zone: v.get("zone").and_then(Value::as_str).map(str::to_owned),
            units: v.get("units").and_then(Value::as_str).and_then(Units::from_name),
        }
    }

    fn save(&self) {
        let v = json!({"place": self.place.as_ref().map(Place::to_json), "chosen": self.chosen, "zone": self.zone, "units": self.units.map(Units::name)});
        crate::sys::write_atomic(&Self::file(), v.to_string().as_bytes()).ok();
    }

    pub fn units(&self, zone: &str) -> Units {
        self.units.unwrap_or_else(|| Units::of_zone(zone))
    }

    /// Where the forecast is for in this zone: the place chosen, or the
    /// zone's city once it has been found for this zone.
    pub fn place_for(&self, zone: &str) -> Option<&Place> {
        self.place.as_ref().filter(|_| self.chosen || self.zone.as_deref() == Some(zone))
    }
}

// -------------------------------------------------------------- forecast ---

/// The weather at a time: the forecast's reading of now, or an hour's.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Reading {
    /// In the place's own time.
    pub time: NaiveDateTime,
    /// Degrees Celsius, and km/h.
    pub temp: f32,
    pub feels: f32,
    pub humidity: Option<f32>,
    pub wind: Option<f32>,
    pub code: u8,
    pub day: bool,
    /// The chance of rain or snow in the hour, in percent.
    pub rain: Option<f32>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Day {
    pub date: NaiveDate,
    pub code: u8,
    pub high: f32,
    pub low: f32,
    pub rain: Option<f32>,
    pub sunrise: Option<NaiveDateTime>,
    pub sunset: Option<NaiveDateTime>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Forecast {
    /// The forecast's reading of now, as it was when fetched.
    pub current: Reading,
    pub hours: Vec<Reading>,
    pub days: Vec<Day>,
    /// The place's offset from UTC, in seconds.
    pub offset: i64,
}

fn local(v: &Value) -> Option<NaiveDateTime> {
    NaiveDateTime::parse_from_str(v.as_str()?, "%Y-%m-%dT%H:%M").ok()
}

fn number(v: Option<&Value>) -> Option<f32> {
    v?.as_f64().filter(|x| x.is_finite()).map(|x| x as f32)
}

fn code(v: Option<&Value>) -> Option<u8> {
    number(v).map(|c| c.clamp(0.0, 255.0) as u8)
}

/// One reading: `current`'s fields, or the `i`th of each of `hourly`'s.
fn reading(o: &Value, i: Option<usize>) -> Option<Reading> {
    let at = |key: &str| match i {
        Some(i) => o.get(key).and_then(|a| a.get(i)),
        None => o.get(key),
    };
    let temp = number(at("temperature_2m"))?;
    Some(Reading {
        time: local(at("time")?)?,
        temp,
        feels: number(at("apparent_temperature")).unwrap_or(temp),
        humidity: number(at("relative_humidity_2m")),
        wind: number(at("wind_speed_10m")),
        code: code(at("weather_code"))?,
        day: number(at("is_day")).is_none_or(|d| d >= 0.5),
        rain: number(at("precipitation_probability")),
    })
}

/// A forecast as Open-Meteo's reply gives it.
pub fn parse(v: &Value) -> Option<Forecast> {
    let current = reading(v.get("current")?, None)?;
    let hourly = v.get("hourly");
    let count = |o: Option<&Value>| o.and_then(|o| o.get("time")).and_then(Value::as_array).map_or(0, Vec::len);
    let hours = hourly.map_or(Vec::new(), |h| (0..count(hourly)).filter_map(|i| reading(h, Some(i))).collect());
    let daily = v.get("daily");
    let days = daily.map_or(Vec::new(), |d| {
        (0..count(daily))
            .filter_map(|i| {
                let at = |key: &str| d.get(key).and_then(|a| a.get(i));
                Some(Day {
                    date: NaiveDate::parse_from_str(at("time")?.as_str()?, "%Y-%m-%d").ok()?,
                    code: code(at("weather_code"))?,
                    high: number(at("temperature_2m_max"))?,
                    low: number(at("temperature_2m_min"))?,
                    rain: number(at("precipitation_probability_max")),
                    sunrise: at("sunrise").and_then(local),
                    sunset: at("sunset").and_then(local),
                })
            })
            .collect()
    });
    Some(Forecast { current, hours, days, offset: v.get("utc_offset_seconds").and_then(Value::as_i64).unwrap_or(0) })
}

fn hour_of(t: NaiveDateTime) -> NaiveDateTime {
    t.date().and_hms_opt(t.hour(), 0, 0).unwrap_or(t)
}

impl Forecast {
    /// The time at the place, `unix` by the handheld's clock.
    pub fn local_time(&self, unix: i64) -> NaiveDateTime {
        chrono::DateTime::from_timestamp(unix + self.offset, 0).map_or(self.current.time, |t| t.naive_utc())
    }

    /// The hours from the one under way at `now`.
    pub fn hours_from(&self, now: NaiveDateTime) -> &[Reading] {
        let first = self.hours.iter().position(|h| h.time >= hour_of(now)).unwrap_or(self.hours.len());
        &self.hours[first..]
    }

    /// The days from `now`'s.
    pub fn days_from(&self, now: NaiveDateTime) -> &[Day] {
        let first = self.days.iter().position(|d| d.date >= now.date()).unwrap_or(self.days.len());
        &self.days[first..]
    }

    /// What it is like at `now`: the forecast's own reading while it is
    /// recent, then the hour's forecast; none once the forecast has run out.
    pub fn at(&self, now: NaiveDateTime) -> Option<Reading> {
        if (now - self.current.time).num_seconds() < CURRENT_FOR {
            return Some(self.current);
        }
        self.hours_from(now).first().copied()
    }

    /// The hours of one day.
    fn hours_of(&self, date: NaiveDate) -> Vec<Reading> {
        self.hours.iter().filter(|h| h.time.date() == date).copied().collect()
    }
}

fn demo_forecast() -> Forecast {
    serde_json::from_str::<Value>(DEMO).ok().as_ref().and_then(parse).expect("the simulator's forecast reads")
}

// --------------------------------------------------------------- fetching ---

/// The last forecast fetched, as kept in `weather.json`.
struct Kept {
    place: Place,
    fetched: i64,
    forecast: Forecast,
}

fn kept_file() -> PathBuf {
    crate::config_dir().join("weather.json")
}

fn keep(place: &Place, fetched: i64, raw: &Value) {
    let v = json!({"place": place.to_json(), "fetched": fetched, "forecast": raw});
    crate::sys::write_atomic(&kept_file(), v.to_string().as_bytes()).ok();
}

fn kept() -> Option<Kept> {
    let v: Value = serde_json::from_str(&std::fs::read_to_string(kept_file()).ok()?).ok()?;
    Some(Kept { place: Place::from_json(v.get("place")?)?, fetched: v.get("fetched")?.as_i64()?, forecast: parse(v.get("forecast")?)? })
}

/// What a fetch brought back.
struct Fetched {
    place: Place,
    /// The zone the place was found for, when it was the zone's city to find.
    found_for: Option<String>,
    forecast: Forecast,
    fetched: i64,
}

/// The forecast for `place`, finding the zone's city first when there is
/// no place yet. Kept once fetched.
fn fetch(place: Option<Place>, zone: &str) -> Result<Fetched, String> {
    let (place, found_for) = match place {
        Some(place) => (place, None),
        None => {
            let city = zone_city(zone).ok_or("no place chosen")?;
            let place = in_zone(&search(&city)?, zone).ok_or_else(|| format!("{} was not found", city.to_lowercase()))?;
            (place, Some(zone.to_owned()))
        }
    };
    let raw = get(&format!("{FORECAST}?latitude={:.4}&longitude={:.4}&{FIELDS}", place.lat, place.lon))?;
    let forecast = parse(&raw).ok_or("the forecast was not readable")?;
    let fetched = now();
    keep(&place, fetched, &raw);
    Ok(Fetched { place, found_for, forecast, fetched })
}

/// Places by name, from the place-name search.
fn search(name: &str) -> Result<Vec<Found>, String> {
    if demo() {
        return Ok(demo_search(name));
    }
    let v = get(&format!("{GEOCODING}?name={}&count=8&language=en&format=json", encode(name.trim())))?;
    Ok(parse_found(&v))
}

/// Text as it may stand in a URL's query.
fn encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// One HTTPS GET with curl, as JSON; curl checks the server's certificate.
fn get(url: &str) -> Result<Value, String> {
    let out = std::process::Command::new("curl")
        .args(["-fsS", "--max-time", "15", "--proto", "=https"])
        .arg(url)
        .output()
        .map_err(|_| "no curl on this system".to_string())?;
    if !out.status.success() {
        let why = String::from_utf8_lossy(&out.stderr);
        return Err(if why.contains("resolve") || why.contains("connect") {
            "offline".into()
        } else if why.contains("429") {
            "asked too often · later".into()
        } else {
            "open-meteo did not answer".into()
        });
    }
    serde_json::from_slice(&out.stdout).map_err(|_| "the reply was not readable".to_string())
}

// ------------------------------------------------------------------ tile ---

thread_local! {
    /// Home's tile line, and when it was read.
    static TILE: RefCell<Option<(Instant, String)>> = const { RefCell::new(None) };
}

/// Home's tile: the temperature and the weather in a word, from the
/// forecast kept. Read from the file once a minute at most.
pub fn tile() -> (String, Option<&'static str>) {
    let line = TILE.with(|tile| {
        let mut tile = tile.borrow_mut();
        match tile.as_ref() {
            Some((at, line)) if at.elapsed() < Duration::from_secs(60) => line.clone(),
            _ => {
                let line = tile_line();
                *tile = Some((Instant::now(), line.clone()));
                line
            }
        }
    });
    (line, None)
}

/// The tile reads the forecast again next time: it, the place, the units or
/// the time zone changed.
pub fn tile_changed() {
    TILE.with(|tile| *tile.borrow_mut() = None);
}

fn tile_line() -> String {
    let prefs = Prefs::load();
    let zone = crate::device::zone();
    let (forecast, at) = if demo() {
        let forecast = demo_forecast();
        let at = forecast.current.time;
        (forecast, at)
    } else {
        match (kept(), prefs.place_for(&zone)) {
            (Some(kept), Some(place)) if kept.place.same(place) => {
                let at = kept.forecast.local_time(now());
                (kept.forecast, at)
            }
            _ => return "–".into(),
        }
    };
    match forecast.at(at) {
        Some(r) => tile_text(&r, prefs.units(&zone)),
        None => "–".into(),
    }
}

fn tile_text(r: &Reading, units: Units) -> String {
    format!("{} {}", units.temp(r.temp), word(r.code, r.day))
}

/// The weather at a glance, for StandBy: now, today's range and the next
/// hours, from the forecast kept (the simulator's own on a computer). Read
/// from the file each time: StandBy asks once a minute at most.
#[derive(Clone, Debug, PartialEq)]
pub struct Glance {
    pub place: String,
    pub temp: String,
    pub condition: &'static str,
    pub icon: &'static str,
    /// "h 27° · l 18°".
    pub range: Option<String>,
    /// The next hours: "15", its icon, its temperature.
    pub hours: Vec<(String, &'static str, String)>,
    /// The chance of rain in the hour, when it is worth saying.
    pub rain: Option<u32>,
}

pub fn glance(h24: bool) -> Option<Glance> {
    let prefs = Prefs::load();
    let zone = crate::device::zone();
    let (forecast, place, at) = if demo() {
        let forecast = demo_forecast();
        let at = forecast.current.time;
        (forecast, demo_place(), at)
    } else {
        let kept = kept()?;
        let place = prefs.place_for(&zone).filter(|p| kept.place.same(p))?.clone();
        let at = kept.forecast.local_time(now());
        (kept.forecast, place, at)
    };
    let units = prefs.units(&zone);
    let r = forecast.at(at)?;
    let range = forecast.days_from(at).first().map(|d| format!("h {} · l {}", units.temp(d.high), units.temp(d.low)));
    let hours = forecast
        .hours_from(at)
        .iter()
        .skip(1)
        .take(4)
        .map(|h| (if h24 { h.time.format("%H").to_string() } else { h.time.format("%-I%P").to_string() }, icon(h.code, h.day), units.temp(h.temp)))
        .collect();
    Some(Glance {
        place: place.name,
        temp: units.temp(r.temp),
        condition: condition(r.code, r.day),
        icon: icon(r.code, r.day),
        range,
        hours,
        rain: r.rain.filter(|p| *p >= 20.0).map(|p| p.round() as u32),
    })
}

// ---------------------------------------------------------------- layout ---

/// The big temperature.
fn temp_style(g: &Gfx) -> TextStyle {
    g.ty.display(30.0)
}

/// Where a page's content ends: above the line along the bottom.
fn bottom(g: &Gfx) -> f32 {
    H - g.hint_h() - 2.0
}

/// The hero's height at this text size.
fn hero_h(g: &Gfx) -> f32 {
    let ty = g.ty;
    let lines = 3.0 * (ty.caption.size + 3.0) + 12.0;
    let left = temp_style(g).size * 0.72 + ty.body.size + 22.0;
    lines.max(left).round()
}

/// The strip's height: the hour, its icon, its temperature.
fn strip_h(g: &Gfx) -> f32 {
    (4.0 + g.ty.caption.size + 2.0 + STRIP_ICON + 2.0 + g.ty.label.size + 4.0).round()
}

/// What the hero says.
struct HeroText {
    temp: String,
    condition: &'static str,
    place: String,
    feels: String,
    wind: String,
    humidity: String,
}

fn hero_text(r: &Reading, units: Units, place: &str) -> HeroText {
    HeroText {
        temp: units.temp(r.temp),
        condition: condition(r.code, r.day),
        place: place.to_owned(),
        feels: format!("feels {}", units.temp(r.feels)),
        wind: r.wind.map_or_else(|| "–".into(), |w| units.wind(w)),
        humidity: r.humidity.map_or_else(|| "–".into(), |h| format!("{}%", h.round() as i32)),
    }
}

/// Where the hero's parts go: the temperature and the condition at the
/// left, three lines at the right (the place, what it feels like, the wind
/// and the humidity), and the room each of those has beside what is at its
/// left.
struct HeroPlan {
    x: f32,
    temp_cy: f32,
    condition_cy: f32,
    lines: [f32; 3],
    right: f32,
    room: [f32; 3],
}

fn hero_plan(g: &mut Gfx, r: Rect, text: &HeroText) -> HeroPlan {
    let ty = g.ty;
    let temp = temp_style(g);
    let x = r.x + 48.0;
    let pitch = ty.caption.size + 3.0;
    let right = r.right() - 10.0;
    let beside_temp = right - (x + g.measure(&text.temp, temp) + 10.0);
    let beside_condition = right - (x + 1.0 + g.measure(text.condition, ty.body) + 10.0);
    HeroPlan {
        x,
        temp_cy: r.y + 8.0 + temp.size * 0.36,
        condition_cy: r.bottom() - 8.0 - ty.body.size * 0.5,
        lines: [r.cy() - pitch, r.cy(), r.cy() + pitch],
        right,
        room: [beside_temp, beside_temp, beside_condition],
    }
}

fn line_icon(g: &Gfx) -> f32 {
    (g.ty.caption.size + 2.0).round()
}

/// The widths of the hero's right-hand lines, as `draw_hero` lays them out.
#[cfg(test)]
fn hero_widths(g: &mut Gfx, text: &HeroText) -> [f32; 3] {
    let (cap, icon) = (g.ty.caption, line_icon(g));
    let place = icon + 3.0 + g.measure(&text.place, cap);
    let feels = g.measure(&text.feels, cap);
    let weather = icon + 2.0 + g.measure(&text.wind, cap) + 8.0 + icon + 2.0 + g.measure(&text.humidity, cap);
    [place, feels, weather]
}

/// The week's columns along a day's row.
struct DayCols {
    icon_cx: f32,
    rain_right: f32,
    low_right: f32,
    bar: (f32, f32),
    high_right: f32,
}

/// A row's name in the look in use: Bezel's rows name things lowercase in
/// the mono voice.
fn row_name(g: &Gfx, name: &str) -> (String, TextStyle) {
    if g.theme == crate::look::Theme::Bezel {
        (name.to_lowercase(), g.ty.label)
    } else {
        (name.to_owned(), g.ty.body)
    }
}

fn day_cols(g: &mut Gfx, r: Rect) -> DayCols {
    let ty = g.ty;
    let name_w = ["Today", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"]
        .iter()
        .map(|n| {
            let (name, style) = row_name(g, n);
            g.measure(&name, style)
        })
        .fold(0.0, f32::max);
    let temp_w = ["−40°", "110°"].iter().map(|s| g.measure(s, ty.label)).fold(0.0, f32::max);
    let rain_w = g.measure("100%", ty.caption);
    let icon_cx = r.x + 10.0 + name_w + 8.0 + DAY_ICON * 0.5;
    let rain_right = icon_cx + DAY_ICON * 0.5 + 4.0 + rain_w;
    let low_right = rain_right + 6.0 + temp_w;
    let high_right = r.right() - 10.0;
    DayCols { icon_cx, rain_right, low_right, bar: (low_right + 7.0, high_right - temp_w - 7.0), high_right }
}

/// An hour as the strip labels it: "14:00", or "2 pm" on a 12-hour clock.
fn hour_label(t: NaiveDateTime, h24: bool) -> String {
    let h = t.hour();
    if h24 {
        format!("{h:02}:00")
    } else {
        format!("{} {}", if h % 12 == 0 { 12 } else { h % 12 }, if h < 12 { "am" } else { "pm" })
    }
}

/// A time of day: "05:40", or "5:40 am".
fn clock(t: NaiveDateTime, h24: bool) -> String {
    if h24 {
        t.format("%H:%M").to_string()
    } else {
        t.format("%-I:%M %p").to_string().to_lowercase()
    }
}

/// A day as its row names it: Today, then Mon, Tue…
fn day_name(date: NaiveDate, today: NaiveDate) -> String {
    if date == today {
        "Today".into()
    } else {
        date.format("%a").to_string()
    }
}

/// And as its page is titled: Today, Tomorrow, then Wed 25… (a whole
/// "Wednesday" is wider than the title's room beside the island at the
/// large text size).
fn day_title(date: NaiveDate, today: NaiveDate) -> String {
    match (date - today).num_days() {
        0 => "Today".into(),
        1 => "Tomorrow".into(),
        _ => date.format("%a %-d").to_string(),
    }
}

/// When a forecast was fetched, by the handheld's clock: "13:15", or
/// "tue 13:15" once it is from another day.
fn as_of(fetched: i64, h24: bool) -> String {
    let Some(at) = chrono::DateTime::from_timestamp(fetched, 0) else { return "–".into() };
    let at = at.with_timezone(&chrono::Local);
    let time = clock(at.naive_local(), h24);
    if at.date_naive() == chrono::Local::now().date_naive() {
        time
    } else {
        format!("{} {time}", at.format("%a").to_string().to_lowercase())
    }
}

// --------------------------------------------------------------- weather ---

#[derive(Clone, Copy, PartialEq, Debug)]
enum Section {
    Now,
    Hours,
    Days,
}

/// The app: now, the hours, and the week.
pub struct Weather {
    prefs: Prefs,
    zone: String,
    /// Where the forecast is for, once known.
    place: Option<Place>,
    forecast: Option<Forecast>,
    /// When it was fetched, in Unix seconds.
    fetched: Option<i64>,
    /// The fetch under way.
    job: Option<Receiver<Result<Fetched, String>>>,
    /// When a fetch was last begun, and why the last one failed.
    asked: Option<Instant>,
    problem: Option<String>,
    demo: bool,
    section: Section,
    /// The hour in focus along the strip, the first in view, and where the
    /// strip has glided to.
    hour: usize,
    strip_first: usize,
    strip: Animated,
    days: ListState,
    /// 0 with now in view, 1 with it folded away for the week.
    fold: Animated,
    /// The place's hour when last drawn: the strip moves on as it turns.
    drawn_hour: Option<NaiveDateTime>,
}

impl Weather {
    pub fn new() -> Self {
        let mut weather = Weather {
            prefs: Prefs::load(),
            zone: crate::device::zone(),
            place: None,
            forecast: None,
            fetched: None,
            job: None,
            asked: None,
            problem: None,
            demo: demo(),
            section: Section::Now,
            hour: 0,
            strip_first: 0,
            strip: Animated::new(0.0, role::hold()).with_rest(0.004),
            days: ListState::default(),
            fold: Animated::new(0.0, role::page()).with_rest(0.004),
            drawn_hour: None,
        };
        weather.load();
        weather
    }

    /// A page on a forecast given, for the tests.
    #[cfg(test)]
    fn showing(forecast: Forecast, place: Place, units: Units) -> Self {
        let mut weather = Weather::new();
        weather.demo = true;
        weather.job = None;
        weather.prefs.units = Some(units);
        weather.place = Some(place);
        weather.forecast = Some(forecast);
        weather.fetched = Some(now());
        weather
    }

    /// Take up the place and the forecast kept for it, and ask for a newer
    /// one if it is due.
    fn load(&mut self) {
        if self.demo {
            self.place = Some(self.prefs.place.clone().filter(|_| self.prefs.chosen).unwrap_or_else(demo_place));
            self.forecast = Some(demo_forecast());
            self.fetched = Some(now());
            return;
        }
        self.place = self.prefs.place_for(&self.zone).cloned();
        (self.forecast, self.fetched) = match (kept(), &self.place) {
            (Some(kept), Some(place)) if kept.place.same(place) => (Some(kept.forecast), Some(kept.fetched)),
            _ => (None, None),
        };
        if self.due() {
            self.ask();
        }
    }

    fn units(&self) -> Units {
        self.prefs.units(&self.zone)
    }

    /// Whether to ask for the forecast now: none kept, or the one kept is
    /// half an hour old; not again straight after a try that failed.
    fn due(&self) -> bool {
        if self.demo || self.job.is_some() || (self.place.is_none() && zone_city(&self.zone).is_none()) {
            return false;
        }
        if self.problem.is_some() && self.asked.is_some_and(|at| at.elapsed() < RETRY) {
            return false;
        }
        self.fetched.is_none_or(|at| now() - at >= REFRESH)
    }

    fn ask(&mut self) {
        let (send, receive) = mpsc::channel();
        let (place, zone) = (self.place.clone(), self.zone.clone());
        let asking = std::thread::Builder::new().name("weather".into()).stack_size(256 * 1024).spawn(move || {
            send.send(fetch(place, &zone)).ok();
        });
        self.asked = Some(Instant::now());
        if asking.is_ok() {
            self.job = Some(receive);
        }
    }

    fn take(&mut self, result: Result<Fetched, String>) {
        let fetched = match result {
            Ok(fetched) => fetched,
            Err(why) => {
                self.problem = Some(why);
                return;
            }
        };
        if let Some(zone) = fetched.found_for {
            // The zone's city, found: kept, so it is found once.
            if zone != self.zone {
                return;
            }
            self.prefs.place = Some(fetched.place.clone());
            self.prefs.zone = Some(zone);
            self.prefs.chosen = false;
            self.prefs.save();
        } else if self.place.as_ref().is_some_and(|p| !p.same(&fetched.place)) {
            return;
        }
        self.place = Some(fetched.place);
        self.forecast = Some(fetched.forecast);
        self.fetched = Some(fetched.fetched);
        self.problem = None;
        tile_changed();
    }

    /// The time at the place: the simulator's forecast keeps its own.
    fn local_now(&self, forecast: &Forecast) -> NaiveDateTime {
        if self.demo {
            forecast.current.time
        } else {
            forecast.local_time(now())
        }
    }

    fn set_section(&mut self, section: Section) {
        self.section = section;
        let want = if section == Section::Days { 1.0 } else { 0.0 };
        if self.fold.target() != want {
            self.fold.set(want);
        }
    }

    /// Along the strip by `by` hours of `count`; at an end it gives a little.
    fn move_hour(&mut self, by: isize, count: usize) {
        let to = self.hour as isize + by;
        if to < 0 || to >= count as isize {
            self.strip.nudge(by as f32 * -2.5);
            return;
        }
        self.hour = to as usize;
        if self.hour < self.strip_first {
            self.strip_first = self.hour;
        } else if self.hour >= self.strip_first + CELLS {
            self.strip_first = self.hour + 1 - CELLS;
        }
        self.strip.set(self.strip_first as f32);
    }

    fn choose_place(&self) -> Nav {
        Nav::Push(Box::new(PlaceScreen::new()))
    }

    fn switch_units(&mut self, cx: &mut Cx) {
        let units = self.units().other();
        self.prefs = Prefs { units: Some(units), ..Prefs::load() };
        self.prefs.save();
        tile_changed();
        *cx.say = Some(format!("{} · {}", units.scale(), units.speed_unit()));
    }

    fn refresh(&mut self, cx: &mut Cx) {
        if self.demo {
            *cx.say = Some("the simulator's forecast is a sample".into());
        } else if self.job.is_none() {
            self.problem = None;
            self.ask();
            *cx.say = Some("asking for the forecast".into());
        }
    }

    /// A day's own page.
    fn open_day(&self, date: NaiveDate) -> Nav {
        let Some(forecast) = &self.forecast else { return Nav::Stay };
        match forecast.days.iter().position(|d| d.date == date) {
            Some(index) => Nav::Push(Box::new(DayScreen::new(forecast.clone(), index, self.units(), self.local_now(forecast)))),
            None => Nav::Stay,
        }
    }

    /// The page with no forecast to show.
    fn draw_empty(&mut self, g: &mut Gfx) {
        let (icon, line, hint) = if self.place.is_none() && zone_city(&self.zone).is_none() {
            ("location_on", "no place yet".to_owned(), "l: choose a place")
        } else if let Some(why) = &self.problem {
            ("cloud", why.clone(), "r: try again · l: place")
        } else {
            let line = match &self.place {
                Some(place) => format!("asking for {}'s forecast", place.name),
                None => "finding the time zone's city".into(),
            };
            ("cloud", line, "l: place")
        };
        let line = g.text.fit(&line, &g.ty.caption, W - 2.0 * PAD);
        g.empty(icon, &line);
        g.hint(hint);
    }
}

impl Screen for Weather {
    fn title(&self) -> &str {
        "Weather"
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        match key {
            Key::Back => return Nav::Pop,
            Key::Char('l') => return self.choose_place(),
            Key::Char('u') => {
                self.switch_units(cx);
                return Nav::Stay;
            }
            Key::Char('r') => {
                self.refresh(cx);
                return Nav::Stay;
            }
            _ => {}
        }
        let Some(forecast) = self.forecast.as_ref() else {
            return if key == Key::Enter { self.choose_place() } else { Nav::Stay };
        };
        let now = self.local_now(forecast);
        let hours = forecast.hours_from(now).len().clamp(1, HOURS);
        let days = forecast.days_from(now).len();
        // The day Enter opens: the hour's (now's is today, whatever the
        // hour under way), or the row's.
        let hour_day = match self.hour {
            0 => now.date(),
            i => forecast.hours_from(now).get(i).map_or(now.date(), |h| h.time.date()),
        };
        let row_day = forecast.days_from(now).get(self.days.focus).map(|d| d.date);
        match (self.section, key) {
            (Section::Now, Key::Down) => self.set_section(Section::Hours),
            (Section::Now, Key::Enter) => return self.choose_place(),
            (Section::Hours, Key::Up) => self.set_section(Section::Now),
            (Section::Hours, Key::Down) if days > 0 => {
                self.days.focus = 0;
                self.set_section(Section::Days);
            }
            (Section::Hours, Key::Left) => self.move_hour(-1, hours),
            (Section::Hours, Key::Right) => self.move_hour(1, hours),
            (Section::Hours, Key::Enter) => return self.open_day(hour_day),
            (Section::Days, Key::Up) if self.days.focus == 0 => self.set_section(Section::Hours),
            (Section::Days, Key::Enter | Key::Right) => {
                if let Some(date) = row_day {
                    return self.open_day(date);
                }
            }
            (Section::Days, other) => crate::screens::list_key(&mut self.days, other, days),
            _ => {}
        }
        Nav::Stay
    }

    fn tick(&mut self, dt: f32) -> bool {
        let mut changed = self.fold.tick(dt) | self.strip.tick(dt);
        if let Some(job) = &self.job {
            match job.try_recv() {
                Ok(result) => {
                    self.job = None;
                    self.take(result);
                    changed = true;
                }
                Err(TryRecvError::Disconnected) => {
                    self.job = None;
                    self.problem = Some("the forecast could not be fetched".into());
                    changed = true;
                }
                Err(TryRecvError::Empty) => {}
            }
        }
        if self.due() {
            self.ask();
            changed = true;
        }
        // The hour turned at the place: the strip begins at the next.
        if let (Some(forecast), Some(drawn)) = (&self.forecast, self.drawn_hour) {
            changed |= hour_of(self.local_now(forecast)) != drawn;
        }
        changed
    }

    fn resume(&mut self) {
        // A place chosen, or the zone changed, since: the forecast is another's.
        let (zone, prefs) = (crate::device::zone(), Prefs::load());
        let before = self.prefs.place_for(&self.zone).cloned();
        let after = prefs.place_for(&zone).cloned();
        let moved = zone != self.zone || before.is_some() != after.is_some() || before.zip(after).is_some_and(|(a, b)| !a.same(&b) || a.name != b.name);
        self.prefs = prefs;
        self.zone = zone;
        if moved {
            self.job = None;
            self.problem = None;
            self.asked = None;
            self.hour = 0;
            self.strip_first = 0;
            self.strip.jump(0.0);
            self.days = ListState::default();
            self.set_section(Section::Now);
            self.load();
        }
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        if g.calm {
            self.fold.jump(self.fold.target());
            self.strip.jump(self.strip.target());
        }
        let Some(forecast) = self.forecast.as_ref() else { return self.draw_empty(g) };
        let now = self.local_now(forecast);
        self.drawn_hour = Some(hour_of(now));
        let Some(reading) = forecast.at(now) else {
            self.problem.get_or_insert_with(|| "the forecast kept has run out".into());
            self.forecast = None;
            return self.draw_empty(g);
        };
        // Now first, then the hours after the one under way.
        let hours: Vec<Reading> = std::iter::once(reading).chain(forecast.hours_from(now).iter().skip(1).copied()).take(HOURS).collect();
        let days: Vec<Day> = forecast.days_from(now).to_vec();
        let week = days.iter().fold((f32::MAX, f32::MIN), |(lo, hi), d| (lo.min(d.low), hi.max(d.high)));
        self.hour = self.hour.min(hours.len() - 1);
        let units = self.units();
        let h24 = g.clock.0;
        let place = self.place.as_ref().map_or_else(String::new, |p| p.name.clone());

        let (top, bottom) = (g.top(), bottom(g));
        let (hero_h, strip_h) = (hero_h(g), strip_h(g));
        let fold = self.fold.value().clamp(0.0, 1.0);
        let shown = 1.0 - fold;

        // Now and the hours go up under the title as the week comes up (now
        // fading as it goes, as About's header does), and come back down.
        let scrolled = fold * (hero_h + GAP + strip_h + GAP);
        let hero = Rect::new(PAD, top - scrolled, W - 2.0 * PAD, hero_h);
        if shown > 0.02 && hero.bottom() > top {
            g.canvas.set_clip(Rect::new(0.0, top, W, hero.bottom() - top));
            draw_hero(g, hero, &hero_text(&reading, units, &place), &reading, shown);
            g.canvas.clear_clip();
            if self.section == Section::Now {
                let radius = g.tile_radius();
                g.focus_ring(hero, radius);
            }
        }

        // The hours, gliding along to keep the one in focus in view.
        let strip = Rect::new(PAD, hero.bottom() + GAP, W - 2.0 * PAD, strip_h);
        if strip.bottom() > top {
            let radius = g.tile_radius();
            let under_title = strip.y.max(top);
            g.canvas.set_clip(Rect::new(strip.x + g.ox, under_title, strip.w, strip.bottom() - under_title));
            g.surface(strip, radius, t.surface1);
            let cell_w = strip.w / CELLS as f32;
            let along = self.strip.value();
            let first = along.floor().max(0.0) as usize;
            for (i, hour) in hours.iter().enumerate().skip(first).take(CELLS + 1) {
                let cell = Rect::new(strip.x + (i as f32 - along) * cell_w, strip.y, cell_w, strip_h);
                let label = if i == 0 { "now".to_owned() } else { hour_label(hour.time, h24) };
                draw_hour(g, cell, &label, hour, units, i == 0);
            }
            g.canvas.clear_clip();
            if self.section == Section::Hours {
                let cell = Rect::new(strip.x + (self.hour as f32 - self.strip_first as f32) * cell_w, strip.y, cell_w, strip_h);
                g.focus_ring(cell.inset(3.0), (radius - 3.0).max(4.0));
            }
        }

        // The week, coming up from below as the rest goes.
        let list_top = strip.bottom() + GAP;
        let row_h = g.row_h();
        if !days.is_empty() && (bottom - list_top >= row_h || fold > 0.0) {
            let rows = self.days.layout(g, days.len(), list_top, row_h, 3.0, bottom);
            g.canvas.set_clip(Rect::new(0.0, list_top - 4.0, W, (bottom - list_top + 8.0).max(0.0)));
            let r0 = Rect::new(PAD, 0.0, W - 2.0 * PAD - 5.0, row_h);
            let cols = day_cols(g, r0);
            for (i, y) in rows {
                let day = &days[i];
                let today = day.date == now.date();
                let r = Rect::new(PAD, y, r0.w, row_h);
                let focused = self.section == Section::Days && i == self.days.focus;
                draw_day(g, r, &cols, day, now.date(), today.then_some(reading.temp), week, units, focused);
            }
            self.days.scrollbar(g, days.len(), list_top, bottom - 2.0);
        }

        // How things stand, else the keys.
        let hint = match (&self.problem, self.fetched) {
            (Some(why), Some(at)) => format!("{why} · as of {}", as_of(at, h24)),
            (None, Some(at)) if self.job.is_some() && chrono::Utc::now().timestamp() - at >= REFRESH => format!("updating · as of {}", as_of(at, h24)),
            _ => match self.section {
                Section::Now => format!("↓: the week · l: place · u: {}", units.other().scale()),
                Section::Hours => "←→: hours · enter: that day".to_owned(),
                Section::Days => format!("enter: the day · u: {}", units.other().scale()),
            },
        };
        g.hint(&hint);
    }
}

/// Now: the weather's icon, the temperature and what it is like at the
/// left; where, what it feels like, the wind and the humidity at the right.
fn draw_hero(g: &mut Gfx, r: Rect, text: &HeroText, reading: &Reading, alpha: f32) {
    let t = g.tokens;
    let ty = g.ty;
    let radius = g.tile_radius();
    g.canvas.rrect(r.offset(g.ox, 0.0), radius, t.surface1, alpha);
    g.icon_alpha(icon(reading.code, reading.day), true, r.x + 25.0, r.cy(), 32.0, t.ink, alpha);
    let plan = hero_plan(g, r, text);
    g.text_alpha(plan.x, plan.temp_cy, &text.temp, temp_style(g), t.ink, alpha);
    g.text_alpha(plan.x + 1.0, plan.condition_cy, text.condition, ty.body, t.ink, alpha);
    let (cap, size) = (ty.caption, line_icon(g));
    // Where: the name cut short only if it could never fit.
    let place = g.text.fit(&text.place, &cap, (plan.room[0] - size - 3.0).max(0.0));
    let w = g.measure(&place, cap);
    let x = plan.right - w;
    g.text_alpha(x, plan.lines[0], &place, cap, t.ink, alpha);
    g.icon_alpha("location_on", true, x - 3.0 - size * 0.5, plan.lines[0], size, t.ink, alpha);
    let w = g.measure(&text.feels, cap);
    g.text_alpha(plan.right - w, plan.lines[1], &text.feels, cap, t.dim, alpha);
    // The wind, then the humidity.
    let cy = plan.lines[2];
    let w = g.measure(&text.humidity, cap);
    let x = plan.right - w;
    g.text_alpha(x, cy, &text.humidity, cap, t.dim, alpha);
    g.icon_alpha("humidity_percentage", false, x - 2.0 - size * 0.5, cy, size, t.dim, alpha);
    let x = x - 2.0 - size - 8.0;
    let w = g.measure(&text.wind, cap);
    g.text_alpha(x - w, cy, &text.wind, cap, t.dim, alpha);
    g.icon_alpha("air", false, x - w - 2.0 - size * 0.5, cy, size, t.dim, alpha);
}

/// One hour along the strip: when, what it is like, how warm.
fn draw_hour(g: &mut Gfx, r: Rect, label: &str, hour: &Reading, units: Units, now: bool) {
    let t = g.tokens;
    let ty = g.ty;
    let cx = r.cx();
    let time_cy = r.y + 4.0 + ty.caption.size * 0.5;
    g.text_at(cx, time_cy, Align::Center, label, ty.caption, if now { t.ink } else { t.dim });
    let icon_cy = r.y + 4.0 + ty.caption.size + 2.0 + STRIP_ICON * 0.5;
    g.icon(icon(hour.code, hour.day), now, cx, icon_cy, STRIP_ICON, t.ink);
    g.text_at(cx, r.bottom() - 4.0 - ty.label.size * 0.5, Align::Center, &units.temp(hour.temp), ty.label, t.ink);
}

/// One day of the week: its name, its weather, the chance of rain, and its
/// low and high on a bar against the whole week's (today's with a dot at now).
#[allow(clippy::too_many_arguments)]
fn draw_day(g: &mut Gfx, r: Rect, cols: &DayCols, day: &Day, today: NaiveDate, now: Option<f32>, week: (f32, f32), units: Units, focused: bool) {
    let t = g.tokens;
    let ty = g.ty;
    let ink = t.ink;
    g.row(r, &day_name(day.date, today), "", ink, focused);
    let cy = r.cy();
    g.icon(icon(day.code, true), false, cols.icon_cx, cy, DAY_ICON, t.ink);
    if let Some(p) = day.rain.filter(|p| *p >= 20.0) {
        g.text_at(cols.rain_right, cy, Align::Right, &format!("{}%", p.round() as i32), ty.caption, t.ice);
    }
    g.text_at(cols.low_right, cy, Align::Right, &units.temp(day.low), ty.label, t.dim);
    g.text_at(cols.high_right, cy, Align::Right, &units.temp(day.high), ty.label, t.ink);
    let (x0, x1) = cols.bar;
    let (lo, hi) = week;
    let span = (hi - lo).max(1.0);
    let at = |v: f32| x0 + ((v - lo) / span).clamp(0.0, 1.0) * (x1 - x0);
    g.surface(Rect::new(x0, cy - 2.0, x1 - x0, 4.0), 2.0, t.meter);
    let (a, b) = (at(day.low), at(day.high));
    g.surface(Rect::new(a, cy - 2.0, (b - a).max(4.0), 4.0), 2.0, t.dim);
    if let Some(v) = now {
        let x = at(v);
        g.dot(x, cy, 3.5, t.surface1);
        g.dot(x, cy, 2.5, t.ink);
    }
}

// ------------------------------------------------------------------ a day ---

/// One day: what it will be like, its temperature through the day with the
/// chance of rain beneath, and its daylight. ←→ go through the week.
pub struct DayScreen {
    forecast: Forecast,
    index: usize,
    units: Units,
    now: NaiveDateTime,
    title: String,
    /// Another day arriving: from the side it came from, to rest.
    slide: Animated,
}

impl DayScreen {
    fn new(forecast: Forecast, index: usize, units: Units, now: NaiveDateTime) -> Self {
        let mut day = DayScreen { forecast, index, units, now, title: String::new(), slide: Animated::new(0.0, role::settle()).with_rest(0.004) };
        day.retitle();
        day
    }

    fn retitle(&mut self) {
        self.title = day_title(self.forecast.days[self.index].date, self.now.date());
    }

    /// The first day there is to see: today.
    fn first(&self) -> usize {
        self.forecast.days.iter().position(|d| d.date >= self.now.date()).unwrap_or(0)
    }

    fn turn(&mut self, by: isize) {
        let to = self.index as isize + by;
        if to < self.first() as isize || to >= self.forecast.days.len() as isize {
            self.slide.nudge(-by as f32 * 1.5);
            return;
        }
        self.index = to as usize;
        self.retitle();
        self.slide.jump(by as f32);
        self.slide.set(0.0);
    }
}

/// The day page's header: its height, and its caption line.
fn day_head_h(g: &Gfx) -> f32 {
    (g.ty.name.size + g.ty.caption.size + 14.0).round()
}

fn day_caption(day: &Day, h24: bool) -> String {
    let rain = day.rain.map_or_else(|| "–".into(), |p| format!("{}%", p.round() as i32));
    match (day.sunrise, day.sunset) {
        (Some(up), Some(down)) => format!("rain {rain} · sun {}–{}", clock(up, h24), clock(down, h24)),
        _ => format!("rain {rain}"),
    }
}

impl Screen for DayScreen {
    fn title(&self) -> &str {
        &self.title
    }

    fn key(&mut self, key: Key, _cx: &mut Cx) -> Nav {
        match key {
            Key::Back => return Nav::Pop,
            Key::Left | Key::Up => self.turn(-1),
            Key::Right | Key::Down => self.turn(1),
            _ => {}
        }
        Nav::Stay
    }

    fn tick(&mut self, dt: f32) -> bool {
        self.slide.tick(dt)
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        let ty = g.ty;
        if g.calm {
            self.slide.jump(0.0);
        }
        let day = self.forecast.days[self.index].clone();
        let h24 = g.clock.0;
        let units = self.units;
        let ox = g.ox;
        g.ox += self.slide.value() * 18.0;
        let top = g.top();
        let head = day_head_h(g);

        // What it will be like, its low and high, rain and daylight.
        g.icon(icon(day.code, true), true, PAD + 15.0, top + head * 0.5 - 2.0, 28.0, t.ink);
        let x = PAD + 36.0;
        let name_cy = top + 3.0 + ty.name.size * 0.5;
        g.text_at(x, name_cy, Align::Left, condition(day.code, true), ty.name, t.ink);
        let low = units.temp(day.low);
        let right = W - PAD - 4.0;
        let low_w = g.text_at(right, name_cy, Align::Right, &low, ty.name, t.dim);
        g.text_at(right - low_w - 8.0, name_cy, Align::Right, &units.temp(day.high), ty.name, t.ink);
        let caption_cy = top + 3.0 + ty.name.size + 5.0 + ty.caption.size * 0.5;
        g.text_at(x + 1.0, caption_cy, Align::Left, &day_caption(&day, h24), ty.caption, t.dim);

        // Through the day.
        let card = Rect::new(PAD, top + head + 2.0, W - 2.0 * PAD, bottom(g) - (top + head + 2.0));
        let radius = g.tile_radius();
        g.surface(card, radius, t.surface1);
        let hours = self.forecast.hours_of(day.date);
        let today = (day.date == self.now.date()).then_some(self.now);
        draw_curve(g, card, &hours, &day, today, units, h24);
        g.ox = ox;
        g.hint("←→: another day · esc: back");
    }
}

/// A day's temperature hour by hour, over its daylight and the chance of
/// rain, with its warmest and coolest marked, and now if it is today.
fn draw_curve(g: &mut Gfx, card: Rect, hours: &[Reading], day: &Day, now: Option<NaiveDateTime>, units: Units, h24: bool) {
    let t = g.tokens;
    let cap = g.ty.caption;
    let (x0, x1) = (card.x + 14.0, card.right() - 14.0);
    let (y0, y1) = (card.y + 7.0 + cap.size + 3.0, card.bottom() - 5.0 - cap.size - 5.0);
    let x_of = |time: NaiveDateTime| x0 + (time.hour() as f32 + time.minute() as f32 / 60.0) / 23.0 * (x1 - x0);
    let ox = g.ox;
    // Daylight: a lighter band from sunrise to sunset.
    if let (Some(up), Some(down)) = (day.sunrise, day.sunset) {
        let (a, b) = (x_of(up).clamp(x0, x1), x_of(down).clamp(x0, x1));
        if b > a {
            g.surface(Rect::new(a, y0 - 3.0, b - a, y1 - y0 + 3.0), 4.0, t.surface2);
        }
    }
    // Hours along the bottom.
    for h in [0, 6, 12, 18] {
        let Some(time) = day.date.and_hms_opt(h, 0, 0) else { continue };
        let label = hour_label(time, h24);
        let w = g.measure(&label, cap);
        let left = (x_of(time) - w * 0.5).clamp(card.x + 6.0, card.right() - 6.0 - w);
        g.text_at(left, card.bottom() - 5.0 - cap.size * 0.5, Align::Left, &label, cap, t.dim);
    }
    if hours.len() < 2 {
        return;
    }
    // The chance of rain, as bars up from the bottom: half the height is certain.
    let step = (x1 - x0) / 23.0;
    for h in hours {
        if let Some(p) = h.rain.filter(|p| *p > 0.0) {
            let tall = (p / 100.0).clamp(0.0, 1.0) * (y1 - y0) * 0.5;
            g.canvas.rrect(Rect::new(x_of(h.time) - step * 0.3 + ox, y1 - tall, step * 0.6, tall), 1.0, t.ice, 0.45);
        }
    }
    let (lo, hi) = hours.iter().fold((f32::MAX, f32::MIN), |(lo, hi), h| (lo.min(h.temp), hi.max(h.temp)));
    // A day that barely changes is drawn as near flat, not stretched.
    let pad = ((4.0 - (hi - lo)) * 0.5).max(0.0);
    let (lo, hi) = (lo - pad, hi + pad);
    let y_of = |v: f32| y1 - (v - lo) / (hi - lo).max(0.1) * (y1 - y0);
    let pts: Vec<(f32, f32)> = hours.iter().map(|h| (x_of(h.time) + ox, y_of(h.temp))).collect();
    let mut area = pts.clone();
    area.push((pts[pts.len() - 1].0, y1));
    area.push((pts[0].0, y1));
    g.canvas.polygon(&area, t.signal, 0.14);
    g.canvas.polyline(&pts, 2.0, t.signal, 1.0);
    // The warmest and the coolest hours, labelled above their points.
    let warmest = hours.iter().enumerate().max_by(|a, b| a.1.temp.total_cmp(&b.1.temp)).map(|(i, _)| i);
    let coolest = hours.iter().enumerate().min_by(|a, b| a.1.temp.total_cmp(&b.1.temp)).map(|(i, _)| i);
    for i in [warmest, coolest].into_iter().flatten() {
        let (x, y) = (pts[i].0 - ox, pts[i].1);
        g.dot(x, y, 2.5, t.ink);
        let label = units.temp(hours[i].temp);
        let w = g.measure(&label, cap);
        let left = (x - w * 0.5).clamp(card.x + 6.0, card.right() - 6.0 - w);
        g.text_at(left, (y - 5.0 - cap.size * 0.5).max(card.y + 4.0 + cap.size * 0.5), Align::Left, &label, cap, t.ink);
    }
    // Now, on today's.
    if let Some(now) = now {
        let x = x_of(now).clamp(x0, x1);
        g.surface(Rect::new(x - 0.5, y0, 1.0, y1 - y0), 0.5, t.dim);
        let at = hours.iter().position(|h| h.time > now).unwrap_or(hours.len() - 1).max(1);
        let (a, b) = (&hours[at - 1], &hours[at]);
        let f = ((now - a.time).num_seconds() as f32 / (b.time - a.time).num_seconds().max(1) as f32).clamp(0.0, 1.0);
        let y = y_of(a.temp + (b.temp - a.temp) * f);
        g.dot(x, y, 4.0, t.surface1);
        g.dot(x, y, 3.0, t.ink);
    }
}

// ----------------------------------------------------------------- place ---

/// What a row of the place list is.
#[derive(Clone, Debug, PartialEq)]
enum Row {
    /// Back to the time zone's city.
    Zone(String),
    Found(Found),
}

/// Choosing where the forecast is for: type a city and Enter searches;
/// the time zone's city is always the first row.
pub struct PlaceScreen {
    field: Field,
    zone: String,
    prefs: Prefs,
    results: Vec<Found>,
    /// What the results are for.
    searched: Option<String>,
    list: ListState,
    /// The focus is on the list rather than the field.
    in_list: bool,
    job: Option<Receiver<Result<Vec<Found>, String>>>,
    problem: Option<String>,
}

impl PlaceScreen {
    pub fn new() -> Self {
        PlaceScreen {
            field: Field::new("search for a place", "", 40),
            zone: crate::device::zone(),
            prefs: Prefs::load(),
            results: Vec::new(),
            searched: None,
            list: ListState::default(),
            in_list: false,
            job: None,
            problem: None,
        }
    }

    fn rows(&self) -> Vec<Row> {
        let zone = zone_city(&self.zone).map(Row::Zone);
        zone.into_iter().chain(self.results.iter().cloned().map(Row::Found)).collect()
    }

    fn search(&mut self, text: String) {
        self.problem = None;
        if demo() {
            self.found(Ok(demo_search(&text)), text);
            return;
        }
        let (send, receive) = mpsc::channel();
        let query = text.clone();
        let asking = std::thread::Builder::new().name("places".into()).stack_size(256 * 1024).spawn(move || {
            send.send(search(&query)).ok();
        });
        if asking.is_ok() {
            self.job = Some(receive);
            self.searched = Some(text);
        }
    }

    fn found(&mut self, result: Result<Vec<Found>, String>, text: String) {
        self.job = None;
        self.searched = Some(text);
        match result {
            Ok(found) => {
                self.results = found;
                if !self.results.is_empty() {
                    // On the first place found, past the time zone's row.
                    self.in_list = true;
                    self.list = ListState::default();
                    self.list.focus = usize::from(zone_city(&self.zone).is_some());
                }
            }
            Err(why) => self.problem = Some(why),
        }
    }

    fn choose(&mut self, cx: &mut Cx) -> Nav {
        let Some(row) = self.rows().into_iter().nth(self.list.focus) else { return Nav::Stay };
        if self.in_use(&row) {
            return Nav::Pop;
        }
        let mut prefs = Prefs::load();
        let name = match row {
            Row::Zone(city) => {
                prefs = Prefs { place: None, chosen: false, zone: None, ..prefs };
                city
            }
            Row::Found(found) => {
                let name = found.place.name.clone();
                prefs = Prefs { place: Some(found.place), chosen: true, zone: None, ..prefs };
                name
            }
        };
        prefs.save();
        tile_changed();
        *cx.say = Some(format!("weather for {name}"));
        Nav::Pop
    }

    /// Whether a row is the place in use.
    fn in_use(&self, row: &Row) -> bool {
        match row {
            Row::Zone(_) => !self.prefs.chosen,
            Row::Found(found) => self.prefs.chosen && self.prefs.place.as_ref().is_some_and(|p| p.same(&found.place)),
        }
    }
}

impl Screen for PlaceScreen {
    fn title(&self) -> &str {
        "Place"
    }

    fn wants_text(&self) -> bool {
        !self.in_list
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        let rows = self.rows().len();
        if self.in_list {
            match key {
                Key::Back => return Nav::Pop,
                Key::Enter => return self.choose(cx),
                Key::Up if self.list.focus == 0 => self.in_list = false,
                // Typing again: back to the field, with what was typed.
                Key::Char(c) if !c.is_control() => {
                    self.in_list = false;
                    self.field.key(key, |_| true);
                }
                Key::Backspace => {
                    self.in_list = false;
                    self.field.key(key, |_| true);
                }
                other => crate::screens::list_key(&mut self.list, other, rows),
            }
            return Nav::Stay;
        }
        match key {
            Key::Down if rows > 0 => {
                self.in_list = true;
                self.list.focus = 0;
            }
            _ => match self.field.key(key, |c| !c.is_control()) {
                Typed::More => {}
                Typed::Done(text) if text.is_empty() => {
                    if rows > 0 {
                        self.in_list = true;
                    }
                }
                Typed::Done(text) => self.search(text),
                Typed::Cancelled => return Nav::Pop,
            },
        }
        Nav::Stay
    }

    fn tick(&mut self, _dt: f32) -> bool {
        let Some(job) = &self.job else { return false };
        match job.try_recv() {
            Ok(result) => {
                let text = self.searched.clone().unwrap_or_default();
                self.found(result, text);
                true
            }
            Err(TryRecvError::Disconnected) => {
                self.job = None;
                self.problem = Some("the search could not be made".into());
                true
            }
            Err(TryRecvError::Empty) => false,
        }
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        let ty = g.ty;
        self.field.draw_at(g, !self.in_list);
        let list_top = g.top() + Field::height(g) + 6.0;
        let bottom = bottom(g);
        let rows = self.rows();
        let row_h = g.row_h();
        for (i, y) in self.list.layout(g, rows.len(), list_top, row_h, 3.0, bottom) {
            let r = Rect::new(PAD, y, W - 2.0 * PAD - 5.0, row_h);
            let (name, value) = match &rows[i] {
                Row::Zone(city) => (city.clone(), if self.in_use(&rows[i]) { "in use".into() } else { "time zone".to_owned() }),
                Row::Found(found) => (found.place.line(), if self.in_use(&rows[i]) { "in use".into() } else { found.place.country.clone() }),
            };
            g.row(r, &name, &value, t.dim, self.in_list && i == self.list.focus);
        }
        self.list.scrollbar(g, rows.len(), list_top, bottom - 2.0);
        // How the search went, under the rows; else whose forecasts they are
        // (Open-Meteo's data is CC BY 4.0: it asks to be credited).
        let said = if self.job.is_some() {
            "searching…".to_owned()
        } else if let Some(why) = &self.problem {
            why.clone()
        } else if let Some(text) = self.searched.as_ref().filter(|_| self.results.is_empty()) {
            format!("nothing found for “{text}”")
        } else {
            CREDIT.to_owned()
        };
        let y = list_top + rows.len() as f32 * (row_h + 3.0) + ty.caption.size;
        if y + ty.caption.size * 0.5 < bottom {
            let said = g.text.fit(&said, &ty.caption, W - 2.0 * PAD);
            g.text_at(W * 0.5, y, Align::Center, &said, ty.caption, t.dim);
        }
        g.hint(if self.in_list { "enter: choose · type to search again" } else { "type a city · enter searches" });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gfx::TextSize;

    fn demo_value() -> Value {
        serde_json::from_str(DEMO).unwrap()
    }

    #[test]
    fn every_code_has_words_an_icon_and_a_word_for_the_tile() {
        for code in CODES {
            for day in [true, false] {
                assert_ne!(condition(code, day), "Unsettled", "{code}");
                assert_ne!(word(code, day), "unsettled", "{code}");
                let name = icon(code, day);
                assert!(bezel::Icons::has(name), "{name} is drawn");
                assert!(bezel::Icons::has(&format!("{name}_fill")), "{name} filled too");
            }
        }
        assert_eq!((condition(0, true), condition(0, false)), ("Sunny", "Clear"));
        assert_eq!((icon(0, true), icon(0, false)), ("sunny", "clear_night"));
        assert_eq!((icon(2, true), icon(2, false)), ("partly_cloudy_day", "partly_cloudy_night"));
        assert_eq!((icon(51, true), icon(65, true), icon(80, false)), ("rainy", "rainy", "rainy"));
        assert_eq!(icon(75, false), "weather_snowy");
        assert_eq!(icon(99, true), "thunderstorm");
        assert_eq!(condition(66, true), "Freezing rain");
        assert_eq!(icon(250, true), "cloud", "a code not known yet is drawn as cloud");
    }

    #[test]
    fn a_reply_from_open_meteo_reads() {
        let f = parse(&demo_value()).unwrap();
        assert_eq!(f.offset, 11 * 3600, "Sydney in summer time");
        assert_eq!(f.current.time, NaiveDate::from_ymd_opt(2026, 11, 21).unwrap().and_hms_opt(13, 30, 0).unwrap());
        assert_eq!((f.current.temp, f.current.feels, f.current.code, f.current.day), (27.1, 31.3, 1, true));
        assert_eq!((f.current.humidity, f.current.wind), (Some(65.0), Some(12.8)));
        assert_eq!(f.hours.len(), 7 * 24);
        assert_eq!(f.hours[0].rain, Some(87.0));
        assert!(!f.hours[0].day && f.hours[13].day, "midnight is night, one in the afternoon day");
        assert_eq!(f.days.len(), 7);
        let first = &f.days[0];
        assert_eq!((first.code, first.high, first.low, first.rain), (80, 27.0, 18.4, Some(87.0)));
        assert_eq!(first.sunrise.map(|t| clock(t, true)).as_deref(), Some("05:40"));
        assert_eq!(first.sunset.map(|t| clock(t, false)).as_deref(), Some("7:41 pm"));
        // From the afternoon: the hour under way first, today first.
        let now = f.current.time;
        assert_eq!(f.hours_from(now)[0].time.hour(), 13);
        assert_eq!(f.days_from(now)[0].date, first.date);
        assert_eq!(f.at(now), Some(f.current));
        // Three hours on, the forecast for that hour stands in.
        let later = now + chrono::Duration::hours(3);
        assert_eq!(f.at(later).map(|r| r.time.hour()), Some(16));
        // A week and a day on, it has run out.
        assert_eq!(f.at(now + chrono::Duration::days(8)), None);
        // What is missing or broken is not read as anything.
        assert!(parse(&json!({"hourly": {}})).is_none());
        assert!(parse(&json!({"current": {"time": "2026-11-21T13:30", "temperature_2m": null, "weather_code": 1}})).is_none());
        let bare = parse(&json!({"current": {"time": "2026-11-21T13:30", "temperature_2m": 4.0, "weather_code": 3}})).unwrap();
        assert_eq!((bare.current.feels, bare.current.humidity, bare.hours.len()), (4.0, None, 0));
    }

    #[test]
    fn units_convert_and_read_as_people_write_them() {
        assert_eq!(Units::Imperial.degrees(100.0), 212.0);
        assert_eq!(Units::Imperial.degrees(-40.0), -40.0);
        assert_eq!(Units::Imperial.temp(27.1), "81°");
        assert_eq!(Units::Metric.temp(27.1), "27°");
        assert_eq!(Units::Metric.temp(-3.6), "−4°", "a true minus");
        assert_eq!(Units::Metric.temp(-0.3), "0°", "never minus nought");
        assert_eq!(Units::Metric.wind(12.8), "13 km/h");
        assert_eq!(Units::Imperial.wind(12.8), "8 mph");
        assert!((Units::Imperial.speed(100.0) - 62.137).abs() < 0.01);
        assert_eq!(Units::of_zone("America/New_York"), Units::Imperial);
        assert_eq!(Units::of_zone("America/Indiana/Indianapolis"), Units::Imperial);
        assert_eq!(Units::of_zone("Pacific/Honolulu"), Units::Imperial);
        assert_eq!(Units::of_zone("America/Toronto"), Units::Metric, "Canada keeps °C");
        assert_eq!(Units::of_zone("Europe/London"), Units::Metric);
        assert_eq!(Units::of_zone("Etc/UTC"), Units::Metric);
        // Switched, it stays switched; not switched, it follows the zone.
        let prefs = Prefs { units: Some(Units::Metric), ..Prefs::default() };
        assert_eq!(prefs.units("America/Chicago"), Units::Metric);
        assert_eq!(Prefs::default().units("America/Chicago"), Units::Imperial);
        assert_eq!(Units::from_name(Units::Imperial.name()), Some(Units::Imperial));
    }

    #[test]
    fn the_time_zone_names_the_city_to_look_up() {
        assert_eq!(zone_city("America/New_York").as_deref(), Some("New York"));
        assert_eq!(zone_city("America/Los_Angeles").as_deref(), Some("Los Angeles"));
        assert_eq!(zone_city("Australia/Sydney").as_deref(), Some("Sydney"));
        assert_eq!(zone_city("Europe/Zurich").as_deref(), Some("Zürich"), "as listed");
        assert_eq!(zone_city("America/Argentina/Buenos_Aires").as_deref(), Some("Buenos Aires"));
        assert_eq!(zone_city("Asia/Kolkata").as_deref(), Some("Kolkata"), "listed as India: the city instead");
        assert_eq!(zone_city("Asia/Shanghai").as_deref(), Some("Shanghai"));
        assert_eq!(zone_city("America/Boise").as_deref(), Some("Boise"), "not listed: the zone's own name");
        assert_eq!(zone_city("Etc/UTC"), None);
        assert_eq!(zone_city("UTC"), None);
        // Of the places a name finds, the one in the zone.
        let reply = json!({"results": [
            {"name": "Sydney", "latitude": 46.15, "longitude": -60.18, "country_code": "ca", "admin1": "Nova Scotia", "timezone": "America/Glace_Bay"},
            {"name": "Sydney", "latitude": -33.87, "longitude": 151.21, "country_code": "AU", "admin1": "New South Wales", "timezone": "Australia/Sydney"},
            {"name": "Nowhere"},
        ]});
        let found = parse_found(&reply);
        assert_eq!(found.len(), 2, "a result with no position is passed over");
        assert_eq!(found[0].place.country, "CA");
        assert_eq!(in_zone(&found, "Australia/Sydney").map(|p| p.region), Some("New South Wales".into()));
        assert_eq!(in_zone(&found, "Europe/Paris").map(|p| p.region), Some("Nova Scotia".into()), "else the first");
        assert!(parse_found(&json!({"generationtime_ms": 0.5})).is_empty(), "nothing found");
        assert_eq!(encode("São Paulo"), "S%C3%A3o%20Paulo");
    }

    #[test]
    fn a_place_follows_the_zone_until_one_is_chosen() {
        let sydney = demo_place();
        let follows = Prefs { place: Some(sydney.clone()), chosen: false, zone: Some("Australia/Sydney".into()), units: None };
        assert_eq!(follows.place_for("Australia/Sydney"), Some(&sydney));
        assert_eq!(follows.place_for("Europe/London"), None, "the zone changed: its city is found again");
        let chosen = Prefs { chosen: true, zone: None, ..follows.clone() };
        assert_eq!(chosen.place_for("Europe/London"), Some(&sydney), "a place chosen stays");
        // Kept and read back.
        let back = Place::from_json(&sydney.to_json()).unwrap();
        assert!(back.same(&sydney) && back == sydney);
        assert!(Place::from_json(&json!({"name": "x", "lat": 95.0, "lon": 0.0})).is_none());
        assert_eq!(sydney.line(), "Sydney · New South Wales");
    }

    #[test]
    fn the_tile_says_how_warm_and_what_it_is_like() {
        let f = parse(&demo_value()).unwrap();
        assert_eq!(tile_text(&f.current, Units::Metric), "27° sunny");
        assert_eq!(tile_text(&f.current, Units::Imperial), "81° sunny");
        // Within a three-across tile at the standard size, whatever the weather.
        let mut g = Gfx::new(bezel::Tokens::dark());
        let room = (W - 2.0 * PAD - 2.0 * GAP) / 3.0 - 16.0;
        for code in CODES {
            for temp in [-40.0, 43.3] {
                let r = Reading { temp, code, day: true, ..f.current };
                let line = tile_text(&r, Units::Imperial);
                assert!(g.measure(&line, g.ty.caption) <= room, "{line}: {} of {room}", g.measure(&line, g.ty.caption));
            }
        }
    }

    /// The worst a page has to fit: the widest words and numbers.
    fn worst() -> (Vec<&'static str>, Vec<String>) {
        let conditions = CODES.iter().flat_map(|&c| [condition(c, true), condition(c, false)]).collect();
        let temps = ["−40°", "110°", "−18°", "100°"].map(String::from).to_vec();
        (conditions, temps)
    }

    #[test]
    fn every_part_fits_at_every_text_size() {
        let mut g = Gfx::new(bezel::Tokens::dark());
        let (conditions, temps) = worst();
        for theme in [crate::look::Theme::Catalyst, crate::look::Theme::Bezel] {
            for size in [TextSize::Compact, TextSize::Standard, TextSize::Large] {
                g.theme = theme;
                g.set_text_size(size);
                let ty = g.ty;
                let at = format!("{theme:?} {size:?}");
                // Now and the hours, one under the other, above the line along the bottom.
                let end = g.top() + hero_h(&g) + GAP + strip_h(&g);
                assert!(end <= bottom(&g), "{at}: the page runs to {end}, past {}", bottom(&g));
                // Now: every line beside what is at its left.
                let r = Rect::new(PAD, g.top(), W - 2.0 * PAD, hero_h(&g));
                for condition in &conditions {
                    for temp in &temps {
                        for (wind, humidity) in [("99 km/h", "100%"), ("62 mph", "100%")] {
                            let text = HeroText { temp: temp.clone(), condition: *condition, place: "San Francisco".into(), feels: format!("feels {temp}"), wind: wind.into(), humidity: humidity.into() };
                            let plan = hero_plan(&mut g, r, &text);
                            let widths = hero_widths(&mut g, &text);
                            for line in 0..3 {
                                assert!(widths[line] <= plan.room[line], "{at}: line {line} of {temp} {condition} {wind}: {} in {}", widths[line], plan.room[line]);
                            }
                        }
                    }
                }
                // The hours: each label within its cell.
                let cell = (W - 2.0 * PAD) / CELLS as f32 - 6.0;
                for label in ["now", "12 pm", "23:00"].iter().map(|s| s.to_string()).chain(temps.iter().cloned()) {
                    let style = if label.ends_with('°') { ty.label } else { ty.caption };
                    assert!(g.measure(&label, style) <= cell, "{at}: {label}");
                }
                // The week: a name, the rain, the low, a bar, the high.
                let row = Rect::new(PAD, 0.0, W - 2.0 * PAD - 5.0, g.row_h());
                let cols = day_cols(&mut g, row);
                assert!(cols.bar.1 - cols.bar.0 >= 40.0, "{at}: the week's bar is {} wide", cols.bar.1 - cols.bar.0);
                // A day: its words, and its low and high, side by side.
                let right = W - PAD - 4.0;
                for condition in &conditions {
                    let words = PAD + 36.0 + g.measure(condition, ty.name);
                    let temps_w = g.measure("110°", ty.name) + 8.0 + g.measure("100°", ty.name);
                    assert!(words + 8.0 <= right - temps_w, "{at}: {condition}");
                }
                let day = Day { date: NaiveDate::from_ymd_opt(2026, 11, 23).unwrap(), code: 80, high: 30.0, low: 20.0, rain: Some(100.0), sunrise: NaiveDate::from_ymd_opt(2026, 11, 23).unwrap().and_hms_opt(5, 40, 0), sunset: NaiveDate::from_ymd_opt(2026, 11, 23).unwrap().and_hms_opt(19, 41, 0) };
                for h24 in [true, false] {
                    let caption = day_caption(&day, h24);
                    assert!(PAD + 37.0 + g.measure(&caption, ty.caption) <= W - PAD, "{at}: {caption}");
                }
                // The day's title beside the island, and the lines along the bottom.
                for title in ["Wed 30", "Tomorrow", "Weather", "Place"] {
                    assert!(g.measure(title, ty.title) <= g.title_room, "{at}: {title}");
                }
                for hint in ["↓: the week · l: place · u: °F", "←→: hours · enter: that day", "enter: the day · u: °F", "←→: another day · esc: back", "enter: choose · type to search again", "offline · as of tue 12:45 pm"] {
                    assert!(g.measure(hint, ty.caption) <= W - 2.0 * PAD, "{at}: {hint}");
                }
            }
        }
    }

    fn press(screen: &mut dyn Screen, key: Key) -> (Nav, Option<String>) {
        let mut robot = Robot::none();
        let mut settings = crate::shell::Settings::default();
        let mut say = None;
        let nav = screen.key(key, &mut Cx { robot: &mut robot, settings: &mut settings, say: &mut say, at_rest: true });
        (nav, say)
    }

    #[test]
    fn the_keys_go_from_now_to_the_hours_to_the_week_and_into_a_day() {
        crate::shell::tests::sandbox();
        let f = parse(&demo_value()).unwrap();
        let mut w = Weather::showing(f, demo_place(), Units::Metric);
        let mut g = Gfx::new(bezel::Tokens::dark());
        w.draw(&mut g, &Robot::none());
        assert_eq!(w.section, Section::Now);
        press(&mut w, Key::Down);
        assert_eq!(w.section, Section::Hours);
        for _ in 0..8 {
            press(&mut w, Key::Right);
        }
        assert_eq!((w.hour, w.strip_first), (8, 3), "the strip keeps the hour in view");
        for _ in 0..40 {
            press(&mut w, Key::Right);
        }
        assert_eq!(w.hour, HOURS - 1, "and stops at its last");
        press(&mut w, Key::Down);
        assert_eq!((w.section, w.days.focus), (Section::Days, 0));
        assert_eq!(w.fold.target(), 1.0, "now folds away for the week");
        press(&mut w, Key::Down);
        let (nav, _) = press(&mut w, Key::Enter);
        let Nav::Push(mut day) = nav else { panic!("enter opens the day") };
        assert_eq!(day.title(), "Tomorrow");
        press(day.as_mut(), Key::Right);
        assert_eq!(day.title(), "Mon 23");
        press(day.as_mut(), Key::Left);
        press(day.as_mut(), Key::Left);
        press(day.as_mut(), Key::Left);
        assert_eq!(day.title(), "Today", "and no further back than today");
        for _ in 0..12 {
            g.tick(1.0 / 60.0);
            day.tick(1.0 / 60.0);
            day.draw(&mut g, &Robot::none());
        }
        // Up from the first day: the hours, then now, which comes back.
        press(&mut w, Key::Up);
        press(&mut w, Key::Up);
        assert_eq!(w.section, Section::Hours);
        assert_eq!(w.fold.target(), 0.0);
        press(&mut w, Key::Up);
        assert_eq!(w.section, Section::Now);
        // Every state draws, at every size, in both looks.
        for size in [TextSize::Compact, TextSize::Standard, TextSize::Large] {
            g.set_text_size(size);
            for key in [Key::Down, Key::Right, Key::Down, Key::Down, Key::Up, Key::Up, Key::Up] {
                press(&mut w, key);
                for _ in 0..20 {
                    g.tick(1.0 / 60.0);
                    w.tick(1.0 / 60.0);
                    w.draw(&mut g, &Robot::none());
                }
            }
        }
    }

    /// Units switched and a place chosen are kept, in one file: one test, so
    /// that no other writes it at the same time.
    #[test]
    fn units_switched_and_a_place_chosen_are_kept() {
        crate::shell::tests::sandbox();
        let f = parse(&demo_value()).unwrap();
        let mut w = Weather::showing(f, demo_place(), Units::Metric);
        let (_, said) = press(&mut w, Key::Char('u'));
        assert_eq!(said.as_deref(), Some("°F · mph"));
        assert_eq!(w.units(), Units::Imperial);
        assert_eq!(Prefs::load().units, Some(Units::Imperial), "kept");
        press(&mut w, Key::Char('u'));
        assert_eq!(Prefs::load().units, Some(Units::Metric));
        // The simulator's search is its own; the handheld's asks the network.
        if !demo() {
            return;
        }
        let mut p = PlaceScreen::new();
        p.zone = "Australia/Sydney".into();
        assert_eq!(p.rows(), vec![Row::Zone("Sydney".into())], "the time zone's city first");
        assert!(p.wants_text());
        for c in "san".chars() {
            press(&mut p, Key::Char(c));
        }
        press(&mut p, Key::Enter);
        assert!(p.in_list && !p.wants_text(), "the results take the focus");
        assert_eq!(p.list.focus, 1, "on the first one found");
        assert_eq!(p.rows().len(), 5);
        press(&mut p, Key::Down);
        let (nav, said) = press(&mut p, Key::Enter);
        assert!(matches!(nav, Nav::Pop));
        assert_eq!(said.as_deref(), Some("weather for San Francisco"));
        let prefs = Prefs::load();
        assert!(prefs.chosen);
        assert_eq!(prefs.place.map(|p| p.name).as_deref(), Some("San Francisco"));
        // Back to following the time zone.
        let mut p = PlaceScreen::new();
        p.zone = "Australia/Sydney".into();
        press(&mut p, Key::Down);
        press(&mut p, Key::Enter);
        let prefs = Prefs::load();
        assert!(!prefs.chosen && prefs.place.is_none());
        // Nothing found says so.
        let mut p = PlaceScreen::new();
        for c in "qq".chars() {
            press(&mut p, Key::Char(c));
        }
        press(&mut p, Key::Enter);
        assert!(p.results.is_empty() && p.searched.as_deref() == Some("qq"));
    }

    /// Against the live services: `cargo test --profile sim -p catalyst-shell
    /// live_open_meteo -- --ignored` (needs the network).
    #[test]
    #[ignore]
    fn live_open_meteo() {
        crate::shell::tests::sandbox();
        std::env::set_var("CATALYST_WEATHER_LIVE", "1");
        let found = search("São Paulo").expect("the place search answers");
        assert!(found.iter().any(|f| f.zone == "America/Sao_Paulo"), "{found:?}");
        let fetched = fetch(None, "America/Chicago").expect("the forecast comes");
        assert_eq!(fetched.place.name, "Chicago");
        assert_eq!(fetched.found_for.as_deref(), Some("America/Chicago"));
        assert!(fetched.forecast.hours.len() >= 7 * 24 && fetched.forecast.days.len() == 7);
        let now = fetched.forecast.local_time(now());
        assert!(fetched.forecast.at(now).is_some(), "it covers now");
        let kept = kept().expect("and is kept");
        assert!(kept.place.same(&fetched.place) && kept.forecast == fetched.forecast);
    }
}
