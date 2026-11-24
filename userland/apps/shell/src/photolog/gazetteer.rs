//! Towns by position, offline: GeoNames' towns of 5,000 people or more
//! (about 70,000), cut to 1.7 MB by tools/gazetteer/build.py and carried in
//! the shell itself. GeoNames' data is under CC BY 4.0
//! (userland/assets/gazetteer/LICENSE-GeoNames.txt).
//!
//! The towns are in order of latitude, so a position looks only at the
//! band half a degree either side of it (a few hundred towns), never the
//! whole list: the pages it touches are few, and the kernel can let them go.

static TOWNS: &[u8] = include_bytes!("../../../../assets/gazetteer/towns.bin");

/// A town found near a position.
#[derive(Clone, Debug, PartialEq)]
pub struct Found {
    /// "Henrietta, NY", "Mississauga, Ontario".
    pub words: String,
    /// How far the position is from it, km.
    pub km: f64,
}

struct Book<'a> {
    n: usize,
    regions: usize,
    data: &'a [u8],
}

const HEAD: usize = 12;
/// Only so far from a town does a place take its name ("near" past [`NEAR`]).
const FARTHEST_KM: f64 = 40.0;
const NEAR: f64 = 8.0;

impl<'a> Book<'a> {
    fn open(data: &'a [u8]) -> Option<Book<'a>> {
        if data.len() < HEAD || &data[..4] != b"CTW1" {
            return None;
        }
        let n = u32::from_le_bytes(data[4..8].try_into().ok()?) as usize;
        let regions = u32::from_le_bytes(data[8..12].try_into().ok()?) as usize;
        (data.len() > HEAD + 9 * n + 3 * regions).then_some(Book { n, regions, data })
    }

    fn i24(&self, at: usize) -> i32 {
        let b = &self.data[at..at + 3];
        i32::from(b[0]) | i32::from(b[1]) << 8 | i32::from(b[2] as i8) << 16
    }

    fn u24(&self, at: usize) -> usize {
        let b = &self.data[at..at + 3];
        usize::from(b[0]) | usize::from(b[1]) << 8 | usize::from(b[2]) << 16
    }

    fn lat(&self, i: usize) -> f64 {
        f64::from(self.i24(HEAD + 3 * i)) / 10_000.0
    }

    fn lon(&self, i: usize) -> f64 {
        f64::from(self.i24(HEAD + 3 * self.n + 3 * i)) / 10_000.0
    }

    fn strings(&self) -> usize {
        HEAD + 9 * self.n + 3 * self.regions
    }

    fn text_at(&self, at: usize) -> &'a str {
        let from = self.strings() + at;
        let end = self.data[from..].iter().position(|&b| b == 0).map_or(self.data.len(), |e| from + e);
        std::str::from_utf8(&self.data[from..end]).unwrap_or("")
    }

    /// Town `i`: its country, its region's name (if any) and its name.
    fn town(&self, i: usize) -> (&'a str, &'a str, &'a str) {
        let at = self.strings() + self.u24(HEAD + 6 * self.n + 3 * i);
        let cc = std::str::from_utf8(&self.data[at..at + 2]).unwrap_or("");
        let region = u16::from_le_bytes([self.data[at + 2], self.data[at + 3]]);
        let region = if usize::from(region) < self.regions { self.text_at(self.u24(HEAD + 9 * self.n + 3 * usize::from(region))) } else { "" };
        let name_from = at + 4 - self.strings();
        (cc, region, self.text_at(name_from))
    }

    /// The first town at `lat` or north of it.
    fn first_from(&self, lat: f64) -> usize {
        let (mut lo, mut hi) = (0, self.n);
        while lo < hi {
            let mid = (lo + hi) / 2;
            if self.lat(mid) < lat {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        lo
    }
}

/// Kilometres between two positions (an equirectangular reckoning: close
/// enough over a few tens of km).
fn km(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    let x = (lon2 - lon1).to_radians() * ((lat1 + lat2) * 0.5).to_radians().cos();
    let y = (lat2 - lat1).to_radians();
    (x * x + y * y).sqrt() * 6371.0
}

/// The nearest town to a position, within [`FARTHEST_KM`].
pub fn nearest(lat: f64, lon: f64) -> Option<Found> {
    nearest_in(TOWNS, lat, lon)
}

fn nearest_in(data: &[u8], lat: f64, lon: f64) -> Option<Found> {
    let book = Book::open(data)?;
    if !lat.is_finite() || !lon.is_finite() {
        return None;
    }
    let band = FARTHEST_KM / 111.0;
    let mut best: Option<(usize, f64)> = None;
    let mut i = book.first_from(lat - band);
    while i < book.n && book.lat(i) <= lat + band {
        let d = km(lat, lon, book.lat(i), book.lon(i));
        if d <= FARTHEST_KM && best.is_none_or(|(_, b)| d < b) {
            best = Some((i, d));
        }
        i += 1;
    }
    let (i, d) = best?;
    let (cc, region, name) = book.town(i);
    let place = if region.is_empty() || region == name {
        format!("{name}, {cc}")
    } else {
        format!("{name}, {region}")
    };
    Some(Found { words: if d > NEAR { format!("near {place}") } else { place }, km: d })
}

/// How many towns it holds.
#[cfg_attr(not(test), allow(dead_code))]
pub fn towns() -> usize {
    Book::open(TOWNS).map_or(0, |b| b.n)
}
