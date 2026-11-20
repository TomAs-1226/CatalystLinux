//! AprilTag detection for the AprilTag reader: AprilRobotics' detector (C,
//! vendored under `c/`, BSD 2-Clause) for the 36h11 family that FRC fields
//! use, with each tag's pose worked out from the camera's lens and the tag's
//! size. The C side is reached only through `c/catalyst_shim.c`, a handful of
//! flat functions, so nothing here mirrors the library's own structs.

use std::os::raw::{c_double, c_float, c_int, c_void};

/// A detection as the shim hands it over.
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct RawTag {
    id: c_int,
    hamming: c_int,
    margin: c_float,
    center: [c_double; 2],
    corners: [[c_double; 2]; 4],
    t: [c_double; 3],
    r: [c_double; 9],
    error: c_double,
    has_pose: c_int,
}

extern "C" {
    fn cat_detector_new(threads: c_int, decimate: c_float) -> *mut c_void;
    fn cat_detector_free(detector: *mut c_void);
    #[allow(clippy::too_many_arguments)]
    fn cat_detect(
        detector: *mut c_void,
        width: c_int,
        height: c_int,
        stride: c_int,
        pixels: *const u8,
        tagsize: c_double,
        fx: c_double,
        fy: c_double,
        cx: c_double,
        cy: c_double,
        out: *mut RawTag,
        max: c_int,
    ) -> c_int;
    fn cat_tag_pixels(id: c_int, out: *mut u8, capacity: c_int, side: *mut c_int) -> c_int;
}

/// The side of an FRC AprilTag's black square since 2024: 6.5 inches.
pub const FRC_TAG_SIZE: f64 = 0.1651;

/// A camera's lens, in pixels of the picture it gives: focal lengths and
/// the optical centre.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Lens {
    pub fx: f64,
    pub fy: f64,
    pub cx: f64,
    pub cy: f64,
}

/// Where a tag is from the camera.
#[derive(Clone, Copy, Debug)]
pub struct Pose {
    /// Metres: to the right of, below, and ahead of the camera.
    pub x: f64,
    pub y: f64,
    pub z: f64,
    /// The rotation from the tag's frame to the camera's, row by row.
    pub rotation: [f64; 9],
    /// The pose's error in object space (smaller is surer).
    pub error: f64,
}

impl Pose {
    /// Straight-line distance to the tag's centre, in metres.
    pub fn distance(&self) -> f64 {
        (self.x * self.x + self.y * self.y + self.z * self.z).sqrt()
    }

    /// How far to the side the tag is, in degrees: positive to the right.
    pub fn bearing(&self) -> f64 {
        self.x.atan2(self.z).to_degrees()
    }

    /// How far the tag is turned from facing the camera square on, about the
    /// vertical, in degrees: positive when its right edge is further away.
    pub fn yaw(&self) -> f64 {
        // The tag's own z axis (into the tag) seen in the camera's frame: it
        // swings toward the camera's left as the tag's right edge goes back.
        let (zx, zz) = (self.rotation[2], self.rotation[8]);
        (-zx).atan2(zz).to_degrees()
    }
}

/// A tag found in a picture.
#[derive(Clone, Debug)]
pub struct Tag {
    pub id: i32,
    /// Bits corrected to read it (0 is a clean read).
    pub hamming: i32,
    /// How clearly its bits stood out (higher is clearer).
    pub margin: f32,
    pub center: (f32, f32),
    /// Its corners in the picture, counter-clockwise from its bottom left.
    pub corners: [(f32, f32); 4],
    /// Where it is, when a lens and a size were given.
    pub pose: Option<Pose>,
}

/// The most tags one picture is read for.
const MOST: usize = 32;

/// An AprilTag detector for the 36h11 family.
pub struct Detector {
    raw: *mut c_void,
    found: Vec<RawTag>,
}

// The C detector is used from one thread at a time (it runs its own worker
// threads inside a call); moving it to another thread between calls is fine.
unsafe impl Send for Detector {}

impl Detector {
    /// `threads` the detector may use; `decimate` how much it shrinks the
    /// picture to look for tag outlines (2 is quick, and still reads tags
    /// across a room at 640 × 480).
    pub fn new(threads: usize, decimate: f32) -> Detector {
        // SAFETY: plain C call; a null comes back only out of memory.
        let raw = unsafe { cat_detector_new(threads.max(1) as c_int, decimate) };
        assert!(!raw.is_null(), "an AprilTag detector");
        Detector { raw, found: vec![RawTag::default(); MOST] }
    }

    /// The tags in a greyscale picture, one byte a pixel, `width` × `height`.
    /// With a lens and a tag size (metres), each comes with its pose.
    pub fn detect(&mut self, pixels: &[u8], width: usize, height: usize, tag_size: f64, lens: Option<Lens>) -> Vec<Tag> {
        if width == 0 || height == 0 || pixels.len() < width * height {
            return Vec::new();
        }
        let (size, l) = match lens {
            Some(l) => (tag_size, l),
            None => (0.0, Lens { fx: 0.0, fy: 0.0, cx: 0.0, cy: 0.0 }),
        };
        // SAFETY: the picture is at least width × height bytes, read only;
        // `found` has room for MOST results, which is what the call is told.
        let n = unsafe {
            cat_detect(self.raw, width as c_int, height as c_int, width as c_int, pixels.as_ptr(), size, l.fx, l.fy, l.cx, l.cy, self.found.as_mut_ptr(), MOST as c_int)
        };
        self.found[..n.clamp(0, MOST as c_int) as usize]
            .iter()
            .map(|r| Tag {
                id: r.id,
                hamming: r.hamming,
                margin: r.margin,
                center: (r.center[0] as f32, r.center[1] as f32),
                corners: r.corners.map(|[x, y]| (x as f32, y as f32)),
                pose: (r.has_pose != 0).then_some(Pose { x: r.t[0], y: r.t[1], z: r.t[2], rotation: r.r, error: r.error }),
            })
            .collect()
    }
}

impl Drop for Detector {
    fn drop(&mut self) {
        // SAFETY: made by cat_detector_new, freed once.
        unsafe { cat_detector_free(self.raw) }
    }
}

/// Tag `id` of the 36h11 family as pixels (0 black, 255 white), with its
/// side in pixels: the white border, the black square, and its bits.
pub fn tag_pixels(id: i32) -> Option<(Vec<u8>, usize)> {
    let mut out = vec![0u8; 64 * 64];
    let mut side: c_int = 0;
    // SAFETY: `out` holds the capacity the call is told.
    let ok = unsafe { cat_tag_pixels(id, out.as_mut_ptr(), out.len() as c_int, &mut side) };
    if ok == 0 || side <= 0 {
        return None;
    }
    let side = side as usize;
    out.truncate(side * side);
    Some((out, side))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A plain grey picture with tag `id` drawn square on, `scale` pixels to
    /// each of the tag's cells, its top left at (x, y).
    fn picture(id: i32, scale: usize, x: usize, y: usize, w: usize, h: usize) -> Vec<u8> {
        let (tag, side) = tag_pixels(id).unwrap();
        let mut px = vec![150u8; w * h];
        for ty in 0..side * scale {
            for tx in 0..side * scale {
                px[(y + ty) * w + x + tx] = tag[(ty / scale) * side + tx / scale];
            }
        }
        px
    }

    #[test]
    fn a_tag_is_drawn_found_and_placed() {
        let (tag, side) = tag_pixels(7).unwrap();
        assert_eq!(side, 10, "36h11: a white border, a black square, 6 × 6 bits");
        assert!(tag.iter().all(|p| *p == 0 || *p == 255));
        // 12 pixels a cell: the black square is 8 cells, 96 pixels across.
        let (w, h) = (320, 240);
        let px = picture(7, 12, 100, 60, w, h);
        let lens = Lens { fx: 300.0, fy: 300.0, cx: 160.0, cy: 120.0 };
        let mut detector = Detector::new(1, 1.0);
        let found = detector.detect(&px, w, h, FRC_TAG_SIZE, Some(lens));
        assert_eq!(found.len(), 1, "one tag");
        let t = &found[0];
        assert_eq!((t.id, t.hamming), (7, 0));
        assert!((t.center.0 - 160.0).abs() < 1.0 && (t.center.1 - 120.0).abs() < 1.0, "{:?}", t.center);
        // Square on, 96 pixels for 0.1651 m at a focal length of 300: 0.516 m away.
        let pose = t.pose.unwrap();
        assert!((pose.z - 300.0 * FRC_TAG_SIZE / 96.0).abs() < 0.02, "{pose:?}");
        assert!(pose.bearing().abs() < 1.0 && pose.yaw().abs() < 5.0, "{pose:?}");
        // No lens: found, no pose.
        assert!(detector.detect(&px, w, h, FRC_TAG_SIZE, None)[0].pose.is_none());
        // Nothing there, nothing found.
        assert!(detector.detect(&vec![150u8; w * h], w, h, FRC_TAG_SIZE, Some(lens)).is_empty());
    }
}
