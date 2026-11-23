//! Finding a page in the camera's preview: its edges, then the largest
//! convex quadrilateral among them.
//!
//! The preview's brightness (320 × 240) is halved and blurred, its edges
//! found (Sobel, kept above a threshold from the frame's own strongest),
//! thickened by a pixel so a page's outline is one piece, and taken apart
//! into connected pieces; so are its bright parts (Otsu's threshold, worn
//! back so a page touching something as bright parts from it). Each large
//! piece's convex hull is worked out, and the four of its corners enclosing
//! the most area found (every pair of opposite corners, the best third and
//! fourth walked along the hull); a piece is kept when that quadrilateral
//! fills its hull (a page's outline is four straight sides). Where the
//! outline runs into other edges (a page on a cluttered bench) its sides are
//! still four lines: the edges' strongest straight lines (a Hough transform)
//! give quadrilaterals too, those whose sides run along edges kept. Each is then
//! checked against the picture at its own size: across each side, at short
//! steps, where the brightness changes most, to a fraction of a pixel; a
//! line fitted to those places; the corners where the lines meet. The page
//! is the largest whose four sides were all found along their length, whole
//! in the picture, large enough, with no corner too sharp.
//!
//! The tests time it (a few milliseconds here; on the A53, reckoned at
//! fifteen times slower, a few tens), on a thread of its own five times a
//! second at most, never the frame's.

/// A point, in pixels.
pub type Pt = (f32, f32);

/// A page's corners: top-left, top-right, bottom-right, bottom-left (as it
/// is seen, clockwise on the panel).
pub type Quad = [Pt; 4];

/// The page's least share of the picture.
const LEAST_AREA: f32 = 0.10;

/// The page in a picture's brightness (`w` × `h`), its corners in its
/// pixels; or none.
pub fn find(luma: &[u8], w: usize, h: usize) -> Option<Quad> {
    if w < 16 || h < 16 || luma.len() < w * h {
        return None;
    }
    // Half size, blurred: the paper's grain and the text's strokes soften.
    let (sw, sh) = (w / 2, h / 2);
    let mut small = vec![0u8; sw * sh];
    for y in 0..sh {
        for x in 0..sw {
            let at = |dx: usize, dy: usize| u32::from(luma[(y * 2 + dy) * w + x * 2 + dx]);
            small[y * sw + x] = ((at(0, 0) + at(1, 0) + at(0, 1) + at(1, 1) + 2) / 4) as u8;
        }
    }
    let blurred = blur(&blur(&small, sw, sh), sw, sh);
    let frame = (sw * sh) as f32;
    let mut candidates: Vec<Quad> = Vec::new();
    // From the edges: each large piece of outline.
    let (thin, normals) = edge_map(&blurred, sw, sh);
    let edges = dilate(&thin, sw, sh);
    candidates.extend(pieces(&edges, sw, sh, (sw + sh) / 2).into_iter().filter_map(|p| quad_of(p, frame)));
    // From the edges' straight lines (where the outline runs into other
    // edges, its piece is no longer a quadrilateral, but its sides are
    // still four lines): the best-held quadrilaterals four of them make.
    candidates.extend(line_quads(&thin, &normals, &edges, sw, sh));
    // From the paper: the bright parts (Otsu's threshold), worn back two
    // pixels so a page touching something as bright comes away from it.
    let bright = erode(&erode(&bright_map(&blurred), sw, sh), sw, sh);
    candidates.extend(pieces(&bright, sw, sh, sw * sh / 20).into_iter().filter_map(|p| quad_of(p, frame)));
    // Each checked against the picture itself, at its own size: its sides
    // found along their whole length, its corners inside the picture.
    let full = blur(luma, w, h);
    let mut best: Option<(f32, Quad)> = None;
    for q in candidates {
        // Back to the picture's pixels (each half-size pixel stood for two).
        let q = order(q).map(|(x, y)| (x * 2.0 + 1.0, y * 2.0 + 1.0));
        let Some(q) = refine(q, &full, w, h) else { continue };
        let inside = q.iter().all(|(x, y)| *x >= 3.0 && *y >= 3.0 && *x <= w as f32 - 3.0 && *y <= h as f32 - 3.0);
        let a = area(&q);
        if !inside || a < (w * h) as f32 * LEAST_AREA || !fair_corners(&q) {
            continue;
        }
        if best.as_ref().is_none_or(|(b, _)| a > *b) {
            best = Some((a, q));
        }
    }
    best.map(|(_, q)| q)
}

/// A piece's quadrilateral: the largest in its hull, if it fills the hull
/// (four straight sides) and the hull is large enough.
fn quad_of(piece: Vec<Pt>, frame: f32) -> Option<Quad> {
    let hull = hull(piece);
    let hull_area = area(&hull);
    if hull.len() < 4 || hull_area < frame * LEAST_AREA {
        return None;
    }
    let q = largest_quad(&hull)?;
    (area(&q) >= hull_area * 0.85 && fair_corners(&q)).then_some(q)
}

/// The brighter part of a picture, by Otsu's threshold.
fn bright_map(src: &[u8]) -> Vec<bool> {
    let mut hist = [0u32; 256];
    for v in src {
        hist[*v as usize] += 1;
    }
    let total = src.len() as f64;
    let sum: f64 = hist.iter().enumerate().map(|(i, c)| i as f64 * f64::from(*c)).sum();
    let (mut below, mut below_sum, mut best, mut at) = (0.0, 0.0, 0.0, 128);
    for (i, c) in hist.iter().enumerate() {
        below += f64::from(*c);
        below_sum += i as f64 * f64::from(*c);
        let above = total - below;
        if below == 0.0 || above == 0.0 {
            continue;
        }
        let (m0, m1) = (below_sum / below, (sum - below_sum) / above);
        let between = below * above * (m0 - m1) * (m0 - m1);
        if between > best {
            best = between;
            at = i;
        }
    }
    src.iter().map(|v| usize::from(*v) > at).collect()
}

/// A map worn back by a pixel each way (4 ways).
fn erode(src: &[bool], w: usize, h: usize) -> Vec<bool> {
    let mut out = vec![false; w * h];
    for y in 1..h.saturating_sub(1) {
        for x in 1..w.saturating_sub(1) {
            let i = y * w + x;
            out[i] = src[i] && src[i - 1] && src[i + 1] && src[i - w] && src[i + w];
        }
    }
    out
}

/// A 1 2 1 blur each way.
fn blur(src: &[u8], w: usize, h: usize) -> Vec<u8> {
    let mut mid = vec![0u16; w * h];
    for y in 0..h {
        for x in 0..w {
            let (l, r) = (x.saturating_sub(1), (x + 1).min(w - 1));
            let row = y * w;
            mid[row + x] = u16::from(src[row + l]) + 2 * u16::from(src[row + x]) + u16::from(src[row + r]);
        }
    }
    let mut out = vec![0u8; w * h];
    for y in 0..h {
        let (u, d) = (y.saturating_sub(1), (y + 1).min(h - 1));
        for x in 0..w {
            out[y * w + x] = ((mid[u * w + x] + 2 * mid[y * w + x] + mid[d * w + x] + 8) / 16) as u8;
        }
    }
    out
}

/// The angle bins lines are told apart by: 2° each, over half a turn.
const BINS: usize = 90;

/// Where the brightness changes sharply: Sobel's |gx| + |gy| over a
/// threshold, a third of the frame's strongest (its 99th centile), never
/// below a floor that grain and noise stay under; and at each such pixel,
/// which way the change runs (the edge's normal, in 2° bins of half a turn).
pub fn edge_map(src: &[u8], w: usize, h: usize) -> (Vec<bool>, Vec<u8>) {
    let mut mag = vec![0u16; w * h];
    let mut normal = vec![0u8; w * h];
    let mut counts = vec![0u32; 2048];
    for y in 1..h - 1 {
        for x in 1..w - 1 {
            let p = |dx: isize, dy: isize| i32::from(src[(y as isize + dy) as usize * w + (x as isize + dx) as usize]);
            let gx = p(1, -1) + 2 * p(1, 0) + p(1, 1) - p(-1, -1) - 2 * p(-1, 0) - p(-1, 1);
            let gy = p(-1, 1) + 2 * p(0, 1) + p(1, 1) - p(-1, -1) - 2 * p(0, -1) - p(1, -1);
            let m = (gx.abs() + gy.abs()).min(2047) as u16;
            mag[y * w + x] = m;
            counts[m as usize] += 1;
            let angle = (gy as f32).atan2(gx as f32).rem_euclid(std::f32::consts::PI);
            normal[y * w + x] = ((angle / std::f32::consts::PI * BINS as f32) as usize % BINS) as u8;
        }
    }
    let total: u32 = counts.iter().sum();
    let mut seen = 0;
    let mut p99 = 2047;
    for (m, c) in counts.iter().enumerate().rev() {
        seen += c;
        if seen * 100 >= total {
            p99 = m;
            break;
        }
    }
    let threshold = (p99 as u16 / 3).max(48);
    (mag.iter().map(|m| *m >= threshold).collect(), normal)
}

/// A line: its normal's angle (radians, 0 to π) and its distance from the
/// picture's corner along it.
#[derive(Clone, Copy, Debug)]
struct Line {
    theta: f32,
    rho: f32,
}

/// Where two lines cross.
fn meet(a: &Line, b: &Line) -> Option<Pt> {
    let (ca, sa, cb, sb) = (a.theta.cos(), a.theta.sin(), b.theta.cos(), b.theta.sin());
    let det = ca * sb - sa * cb;
    (det.abs() > 1e-3).then(|| ((a.rho * sb - b.rho * sa) / det, (ca * b.rho - cb * a.rho) / det))
}

/// The share of a side, from `p` to `r`, that runs along edges.
fn held(edges: &[bool], w: usize, h: usize, p: Pt, r: Pt) -> f32 {
    let len = ((r.0 - p.0).powi(2) + (r.1 - p.1).powi(2)).sqrt();
    let n = (len as usize).max(4);
    let on = (0..n)
        .filter(|k| {
            let t = 0.05 + 0.9 * (*k as f32 + 0.5) / n as f32;
            let (x, y) = (p.0 + (r.0 - p.0) * t, p.1 + (r.1 - p.1) * t);
            x >= 0.0 && y >= 0.0 && (x as usize) < w && (y as usize) < h && edges[y as usize * w + x as usize]
        })
        .count();
    on as f32 / n as f32
}

/// Quadrilaterals from the edges' straight lines: a Hough transform (each
/// edge pixel voting only near its own normal's angle), its strongest twenty
/// lines (each apart from the others), and of the quadrilaterals two pairs
/// of near-opposite lines make, those whose sides run along edges for most
/// of their length, the best few (by area and how well held).
fn line_quads(thin: &[bool], normals: &[u8], edges: &[bool], w: usize, h: usize) -> Vec<Quad> {
    let diag = ((w * w + h * h) as f32).sqrt().ceil() as usize;
    let rhos = 2 * diag + 1;
    let mut votes = vec![0u32; BINS * rhos];
    let trig: Vec<(f32, f32)> = (0..BINS).map(|b| ((b as f32 + 0.5) / BINS as f32 * std::f32::consts::PI).sin_cos()).map(|(s, c)| (c, s)).collect();
    for y in 0..h {
        for x in 0..w {
            if !thin[y * w + x] {
                continue;
            }
            let n = normals[y * w + x] as isize;
            for d in -3..=3isize {
                let b = (n + d).rem_euclid(BINS as isize) as usize;
                let (c, s) = trig[b];
                let rho = (x as f32 + 0.5) * c + (y as f32 + 0.5) * s;
                votes[b * rhos + (rho.round() as isize + diag as isize) as usize] += 1;
            }
        }
    }
    // The strongest lines, none within 6° and 4 pixels of a stronger one.
    let mut peaks: Vec<(u32, usize, usize)> = Vec::new();
    for b in 0..BINS {
        for r in 1..rhos - 1 {
            let v = votes[b * rhos + r];
            if v >= (w.min(h) as u32) / 6 && v >= votes[b * rhos + r - 1] && v >= votes[b * rhos + r + 1] {
                peaks.push((v, b, r));
            }
        }
    }
    peaks.sort_by(|a, b| b.0.cmp(&a.0));
    let mut lines: Vec<Line> = Vec::new();
    for (_, b, r) in peaks {
        let near = |l: &Line| {
            let db = ((l.theta / std::f32::consts::PI * BINS as f32) as isize - b as isize).rem_euclid(BINS as isize);
            let db = db.min(BINS as isize - db);
            db <= 3 && (l.rho - (r as f32 - diag as f32)).abs() <= 4.0
        };
        if lines.iter().any(near) {
            continue;
        }
        lines.push(Line { theta: (b as f32 + 0.5) / BINS as f32 * std::f32::consts::PI, rho: r as f32 - diag as f32 });
        if lines.len() == 20 {
            break;
        }
    }
    // Pairs of near-opposite sides: within 40° of each other.
    let apart = |a: &Line, b: &Line| {
        let d = (a.theta - b.theta).abs();
        d.min(std::f32::consts::PI - d)
    };
    let mut pairs = Vec::new();
    for i in 0..lines.len() {
        for j in i + 1..lines.len() {
            if apart(&lines[i], &lines[j]) < 40f32.to_radians() {
                pairs.push((i, j));
            }
        }
    }
    let frame = (w * h) as f32;
    let mut found: Vec<(f32, Quad)> = Vec::new();
    for (k, &(a, b)) in pairs.iter().enumerate() {
        for &(c, d) in &pairs[k + 1..] {
            if [a, b].iter().any(|i| *i == c || *i == d) || apart(&lines[a], &lines[c]) < 30f32.to_radians() {
                continue;
            }
            let corners = [meet(&lines[a], &lines[c]), meet(&lines[c], &lines[b]), meet(&lines[b], &lines[d]), meet(&lines[d], &lines[a])];
            let Some(q) = corners.into_iter().collect::<Option<Vec<Pt>>>() else { continue };
            let q = order([q[0], q[1], q[2], q[3]]);
            let a_ = area(&q);
            let inside = q.iter().all(|(x, y)| *x >= 1.0 && *y >= 1.0 && *x <= w as f32 - 1.0 && *y <= h as f32 - 1.0);
            if !inside || a_ < frame * LEAST_AREA || !fair_corners(&q) || hull(q.to_vec()).len() != 4 {
                continue;
            }
            let support = (0..4).map(|i| held(edges, w, h, q[i], q[(i + 1) % 4])).fold(1.0f32, f32::min);
            if support >= 0.55 {
                found.push((a_ * support, q));
            }
        }
    }
    found.sort_by(|a, b| b.0.total_cmp(&a.0));
    found.into_iter().take(4).map(|(_, q)| q).collect()
}

/// Edges thickened by a pixel each way, so an outline's small gaps close.
fn dilate(src: &[bool], w: usize, h: usize) -> Vec<bool> {
    let mut out = vec![false; w * h];
    for y in 0..h {
        for x in 0..w {
            if src[y * w + x] {
                for yy in y.saturating_sub(1)..=(y + 1).min(h - 1) {
                    for xx in x.saturating_sub(1)..=(x + 1).min(w - 1) {
                        out[yy * w + xx] = true;
                    }
                }
            }
        }
    }
    out
}

/// The connected pieces (8 ways) of at least `least` pixels, each as its pixels.
fn pieces(map: &[bool], w: usize, h: usize, least: usize) -> Vec<Vec<Pt>> {
    let mut seen = vec![false; w * h];
    let mut out = Vec::new();
    let mut stack = Vec::new();
    for start in 0..w * h {
        if !map[start] || seen[start] {
            continue;
        }
        seen[start] = true;
        stack.push(start);
        let mut piece = Vec::new();
        while let Some(i) = stack.pop() {
            let (x, y) = (i % w, i / w);
            piece.push((x as f32, y as f32));
            for yy in y.saturating_sub(1)..=(y + 1).min(h - 1) {
                for xx in x.saturating_sub(1)..=(x + 1).min(w - 1) {
                    let j = yy * w + xx;
                    if map[j] && !seen[j] {
                        seen[j] = true;
                        stack.push(j);
                    }
                }
            }
        }
        if piece.len() >= least {
            out.push(piece);
        }
    }
    out
}

fn cross(o: Pt, a: Pt, b: Pt) -> f32 {
    (a.0 - o.0) * (b.1 - o.1) - (a.1 - o.1) * (b.0 - o.0)
}

/// The convex hull (Andrew's monotone chain), its corners in turn.
pub fn hull(mut pts: Vec<Pt>) -> Vec<Pt> {
    pts.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
    pts.dedup();
    if pts.len() < 3 {
        return pts;
    }
    let mut lower: Vec<Pt> = Vec::new();
    for p in &pts {
        while lower.len() >= 2 && cross(lower[lower.len() - 2], lower[lower.len() - 1], *p) <= 0.0 {
            lower.pop();
        }
        lower.push(*p);
    }
    let mut upper: Vec<Pt> = Vec::new();
    for p in pts.iter().rev() {
        while upper.len() >= 2 && cross(upper[upper.len() - 2], upper[upper.len() - 1], *p) <= 0.0 {
            upper.pop();
        }
        upper.push(*p);
    }
    lower.pop();
    upper.pop();
    lower.extend(upper);
    lower
}

/// A polygon's area (its corners in turn, either way round).
pub fn area(poly: &[Pt]) -> f32 {
    let n = poly.len();
    (0..n).map(|i| poly[i].0 * poly[(i + 1) % n].1 - poly[(i + 1) % n].0 * poly[i].1).sum::<f32>().abs() * 0.5
}

fn tri(a: Pt, b: Pt, c: Pt) -> f32 {
    cross(a, b, c).abs() * 0.5
}

/// The four of a convex hull's corners that enclose the most: for each pair
/// of opposite corners (i, k), the best corner between them on each side,
/// walked forward as k moves on (the best moves only forward on a convex
/// hull), so the whole search is n² steps.
pub fn largest_quad(hull: &[Pt]) -> Option<Quad> {
    let n = hull.len();
    if n < 4 {
        return None;
    }
    let at = |i: usize| hull[i % n];
    let mut best = (0.0f32, [0usize; 4]);
    for i in 0..n {
        let (mut j, mut l) = (i + 1, i + 3);
        for k in i + 2..i + n - 1 {
            if j >= k {
                j = k - 1;
            }
            while j + 1 < k && tri(at(i), at(j + 1), at(k)) >= tri(at(i), at(j), at(k)) {
                j += 1;
            }
            if l <= k {
                l = k + 1;
            }
            while l + 1 < i + n && tri(at(k), at(l + 1), at(i)) >= tri(at(k), at(l), at(i)) {
                l += 1;
            }
            let a = tri(at(i), at(j), at(k)) + tri(at(k), at(l), at(i));
            if a > best.0 {
                best = (a, [i % n, j % n, k % n, l % n]);
            }
        }
    }
    (best.0 > 0.0).then(|| best.1.map(|i| hull[i]))
}

/// No corner sharper than 45° or blunter than 135°: a page seen at a slant,
/// not a sliver.
fn fair_corners(q: &Quad) -> bool {
    (0..4).all(|i| {
        let (p, c, nx) = (q[(i + 3) % 4], q[i], q[(i + 1) % 4]);
        let (ax, ay, bx, by) = (p.0 - c.0, p.1 - c.1, nx.0 - c.0, nx.1 - c.1);
        let cos = (ax * bx + ay * by) / ((ax * ax + ay * ay).sqrt() * (bx * bx + by * by).sqrt()).max(1e-6);
        cos.abs() < 45f32.to_radians().cos()
    })
}

/// Corners in the page's order: clockwise on the panel from the top-left
/// (the one nearest the picture's top-left corner).
pub fn order(q: Quad) -> Quad {
    let (cx, cy) = (q.iter().map(|p| p.0).sum::<f32>() / 4.0, q.iter().map(|p| p.1).sum::<f32>() / 4.0);
    let mut by_angle = q;
    by_angle.sort_by(|a, b| (a.1 - cy).atan2(a.0 - cx).total_cmp(&(b.1 - cy).atan2(b.0 - cx)));
    let first = (0..4).min_by(|a, b| (by_angle[*a].0 + by_angle[*a].1).total_cmp(&(by_angle[*b].0 + by_angle[*b].1))).unwrap_or(0);
    [0, 1, 2, 3].map(|i| by_angle[(first + i) % 4])
}

/// A picture's brightness at (x, y), read between pixels.
fn sample(luma: &[u8], w: usize, h: usize, x: f32, y: f32) -> f32 {
    let (x, y) = ((x - 0.5).clamp(0.0, (w - 1) as f32), (y - 0.5).clamp(0.0, (h - 1) as f32));
    let (x0, y0) = (x as usize, y as usize);
    let (x1, y1) = ((x0 + 1).min(w - 1), (y0 + 1).min(h - 1));
    let (kx, ky) = (x - x0 as f32, y - y0 as f32);
    let p = |xx: usize, yy: usize| f32::from(luma[yy * w + xx]);
    let top = p(x0, y0) + (p(x1, y0) - p(x0, y0)) * kx;
    let bottom = p(x0, y1) + (p(x1, y1) - p(x0, y1)) * kx;
    top + (bottom - top) * ky
}

/// How far either side of a side its edge is looked for (pixels).
const REACH: i32 = 8;

/// Each side found in the picture at its own size: along it, at short
/// steps, the brightness across it and where that changes most (to a
/// fraction of a pixel); the side fitted to those places (twice, the
/// second time without those far off the first line), and the corners put
/// where the fitted sides meet. None unless every side was found along
/// most of its length (an outline of four real sides, not a guess).
fn refine(q: Quad, luma: &[u8], w: usize, h: usize) -> Option<Quad> {
    let mut lines = [(0.0f32, 0.0f32, 0.0f32); 4];
    for (i, line) in lines.iter_mut().enumerate() {
        let (p, r) = (q[i], q[(i + 1) % 4]);
        let (dx, dy) = (r.0 - p.0, r.1 - p.1);
        let len = (dx * dx + dy * dy).sqrt();
        if len < 8.0 {
            return None;
        }
        let (ux, uy) = (dx / len, dy / len);
        let (nx, ny) = (-uy, ux);
        let steps = ((len / 3.0) as usize).clamp(8, 90);
        let mut pts = Vec::with_capacity(steps);
        let mut across = [0f32; (2 * REACH + 3) as usize];
        for k in 0..steps {
            let t = 0.08 + 0.84 * (k as f32 + 0.5) / steps as f32;
            let (sx, sy) = (p.0 + dx * t, p.1 + dy * t);
            for (j, v) in across.iter_mut().enumerate() {
                let d = j as f32 - (REACH + 1) as f32;
                *v = sample(luma, w, h, sx + nx * d, sy + ny * d);
            }
            // The change across, at each place: the strongest, and either side of it.
            let g = |d: i32| (across[(d + REACH + 2) as usize] - across[(d + REACH) as usize]).abs();
            let (mut at, mut most) = (0, 0.0);
            for d in -REACH..=REACH {
                if g(d) > most {
                    (at, most) = (d, g(d));
                }
            }
            if most < 14.0 {
                continue;
            }
            let mut off = at as f32;
            if at > -REACH && at < REACH {
                let (a, b, c) = (g(at - 1), g(at), g(at + 1));
                let curve = a - 2.0 * b + c;
                if curve.abs() > 1e-3 {
                    off += (0.5 * (a - c) / curve).clamp(-0.5, 0.5);
                }
            }
            pts.push((sx + nx * off, sy + ny * off));
        }
        if pts.len() * 10 < steps * 6 {
            return None;
        }
        // The line most of the places agree on (each through a place and the
        // one half the side along: the most within a pixel of it), then
        // fitted to those alone, so a shadow, a finger or a line of text
        // beside the edge pulls nothing.
        let half = pts.len() / 2;
        let mut best = (0, (0.0, 0.0, 0.0));
        for k in 0..half {
            let Some(l) = fit_line(&[pts[k], pts[k + half]]) else { continue };
            let agree = pts.iter().filter(|(x, y)| (l.0 * x + l.1 * y - l.2).abs() <= 1.0).count();
            if agree > best.0 {
                best = (agree, l);
            }
        }
        let l = best.1;
        let near: Vec<Pt> = pts.iter().copied().filter(|(x, y)| (l.0 * x + l.1 * y - l.2).abs() <= 1.0).collect();
        if near.len() * 2 < pts.len() {
            return None;
        }
        let fitted = fit_line(&near)?;
        if (fitted.0 * nx + fitted.1 * ny).abs() < 0.94 {
            return None;
        }
        *line = fitted;
    }
    let mut out = q;
    for (i, corner) in out.iter_mut().enumerate() {
        // Corner i is where side i−1 (into it) meets side i (out of it).
        let (a, b) = (lines[(i + 3) % 4], lines[i]);
        let det = a.0 * b.1 - a.1 * b.0;
        if det.abs() < 1e-4 {
            return None;
        }
        let p = ((a.2 * b.1 - a.1 * b.2) / det, (a.0 * b.2 - a.2 * b.0) / det);
        if (p.0 - q[i].0).abs() > 2.0 * REACH as f32 || (p.1 - q[i].1).abs() > 2.0 * REACH as f32 {
            return None;
        }
        *corner = p;
    }
    Some(out)
}

/// The line through points that strays least from them (total least
/// squares): `n · x = c`, `n` of unit length.
fn fit_line(pts: &[Pt]) -> Option<(f32, f32, f32)> {
    if pts.len() < 2 {
        return None;
    }
    let n = pts.len() as f32;
    let (mx, my) = (pts.iter().map(|p| p.0).sum::<f32>() / n, pts.iter().map(|p| p.1).sum::<f32>() / n);
    let (mut sxx, mut sxy, mut syy) = (0.0, 0.0, 0.0);
    for (x, y) in pts {
        sxx += (x - mx) * (x - mx);
        sxy += (x - mx) * (y - my);
        syy += (y - my) * (y - my);
    }
    let angle = 0.5 * (2.0 * sxy).atan2(sxx - syy);
    let (nx, ny) = (-angle.sin(), angle.cos());
    Some((nx, ny, nx * mx + ny * my))
}

/// How far apart two pages' corners are, at most (pixels).
pub fn moved(a: &Quad, b: &Quad) -> f32 {
    a.iter().zip(b).map(|(p, q)| ((p.0 - q.0).powi(2) + (p.1 - q.1).powi(2)).sqrt()).fold(0.0, f32::max)
}
