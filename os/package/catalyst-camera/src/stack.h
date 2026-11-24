// stack.h: Night and HDR (1.9), frames merged as they come, and every photo's
// merge and finish (1.5.2), kept apart from
// the camera so it can be checked without one (test/stack-test.cpp, under
// qemu).
//
// Night takes a run of short frames at one exposure (an eighth of a second
// or less in the hand, up to a second braced) and HDR a few frames short
// enough to keep the sky; both are merged here, one frame at a time, into a
// running sum, so no more than one frame is ever held besides the sums:
//
// - Alignment. Each frame against the first, as one shift for the whole
//   picture: found on its brightness at a sixteenth of the size (a wide
//   search), refined at a quarter, then on patches at full size against the
//   merge so far. A frame that will not line up is left out whole.
// - Rejection. Each pixel is weighed against the merge so far: within three
//   times the noise at its brightness it counts in full, fading to nothing
//   at six. What moved never smears: the first frame stands there alone.
//   The noise is measured, not assumed: from the second frame's differences
//   to the first, by brightness, with the median (so what moved between
//   them does not count).
// - Finish (in the photo's writing, after the sums are gone): a gentle
//   noise filter (luma 3 x 3, colour 5 x 5, each only among neighbours
//   within the noise), a gentle pull of the colour toward neutral where it
//   is near grey (night's lamps without the orange or green), then the tone:
//   the brightness the frames could not reach given, a shoulder that keeps
//   the highlights, the shadows lifted a little.
//
// Memory, at w x h: the frame copied out (1.5 bytes a pixel), the luma's sum
// and weight (3), and the colour's (1.25): 5.75 bytes a pixel, 46 MB at
// 3280 x 2464 and 12 MB at 1640 x 1232 (still.h's stack_mb).
//
// 1.5.2, every photo: three frames back to back merged into the first as
// it is written (merge_frames: the frames held, 36 MB at full size, no
// sums), and a new finish for every photo, Night's and HDR's too: its own
// sharpening on the luma (sharpen), in place of most of the ISP's, and a
// local tone (gain_map: the exposure set region by region, at a block's
// size, and spread onto each pixel by its own brightness, so no halo).

#pragma once

#include <algorithm>
#include <cmath>
#include <cstdint>
#include <cstdlib>
#include <cstring>
#include <thread>
#include <vector>

namespace stack {

// Chroma rows [0, rows) shared out among `threads` (each takes the two luma
// rows of its chroma rows).
template <class F>
void in_bands(unsigned rows, unsigned threads, const F &f)
{
	threads = std::max(1u, std::min(threads, rows));
	unsigned step = (rows + threads - 1) / threads;
	std::vector<std::thread> pool;
	for (unsigned t = 1; t < threads; t++) {
		unsigned a = t * step, b = std::min(rows, a + step);
		if (a < b)
			pool.emplace_back([&f, a, b] { f(a, b); });
	}
	f(0, std::min(rows, step));
	for (std::thread &t : pool)
		t.join();
}

// A plane of brightness, small.
struct Plane {
	unsigned w = 0, h = 0;
	std::vector<uint8_t> px;
	uint8_t at(unsigned x, unsigned y) const { return px[size_t(y) * w + x]; }
};

// The mean of each k x k block of a w x h plane, rows `stride` apart.
inline Plane shrink(const uint8_t *src, unsigned w, unsigned h, unsigned stride, unsigned k)
{
	Plane p;
	p.w = w / k;
	p.h = h / k;
	p.px.resize(size_t(p.w) * p.h);
	std::vector<uint32_t> row(p.w);
	for (unsigned by = 0; by < p.h; by++) {
		std::fill(row.begin(), row.end(), 0);
		for (unsigned dy = 0; dy < k; dy++) {
			const uint8_t *s = src + size_t(by * k + dy) * stride;
			for (unsigned bx = 0; bx < p.w; bx++) {
				uint32_t sum = 0;
				for (unsigned dx = 0; dx < k; dx++)
					sum += s[bx * k + dx];
				row[bx] += sum;
			}
		}
		for (unsigned bx = 0; bx < p.w; bx++)
			p.px[size_t(by) * p.w + bx] = uint8_t(row[bx] / (k * k));
	}
	return p;
}

// A frame's brightness at a quarter and a sixteenth of its size.
struct Pyramid {
	Plane quarter, sixteenth;
};

inline Pyramid pyramid(const uint8_t *y, unsigned w, unsigned h)
{
	Pyramid p;
	p.quarter = shrink(y, w, h, w, 4);
	p.sixteenth = shrink(p.quarter.px.data(), p.quarter.w, p.quarter.h, p.quarter.w, 4);
	return p;
}

// How far one picture's content is from another's: b at (x + dx, y + dy)
// against a at (x, y), the mean absolute difference over what both cover
// (a margin in from a's edges), every `step`th pixel.
inline float sad(const Plane &a, const Plane &b, int dx, int dy, unsigned margin, unsigned step)
{
	uint64_t sum = 0, n = 0;
	int x0 = std::max<int>(margin, -dx), x1 = std::min<int>(int(a.w) - int(margin), int(b.w) - dx);
	int y0 = std::max<int>(margin, -dy), y1 = std::min<int>(int(a.h) - int(margin), int(b.h) - dy);
	for (int y = y0; y < y1; y += int(step)) {
		const uint8_t *ra = &a.px[size_t(y) * a.w], *rb = &b.px[size_t(y + dy) * b.w + dx];
		for (int x = x0; x < x1; x += int(step)) {
			sum += unsigned(std::abs(int(ra[x]) - int(rb[x])));
			n++;
		}
	}
	// Too little overlap to judge: as bad as can be.
	if (n < 64)
		return 1e9f;
	return float(sum) / float(n);
}

// A frame's shift against the first, in full-size pixels: its content at
// (x + dx, y + dy) is the first's at (x, y). `err` is the mean difference
// at the shift found (on the quarter-size brightness).
struct Shift {
	int dx = 0, dy = 0;
	float err = 0;
};

// Searched around (cx, cy) by `reach` each way on one level.
inline Shift search(const Plane &a, const Plane &b, int cx, int cy, int reach, unsigned margin, unsigned step)
{
	Shift best;
	best.err = 1e9f;
	for (int dy = cy - reach; dy <= cy + reach; dy++)
		for (int dx = cx - reach; dx <= cx + reach; dx++) {
			float e = sad(a, b, dx, dy, margin, step);
			// Ties go to the smaller shift, so a flat picture stays put.
			if (e < best.err - 1e-4f || (std::fabs(e - best.err) <= 1e-4f && std::abs(dx) + std::abs(dy) < std::abs(best.dx) + std::abs(best.dy))) {
				best.err = e;
				best.dx = dx;
				best.dy = dy;
			}
		}
	return best;
}

// The furthest a hand drifts in a night photo's second, at a sixteenth of
// the size: 8 there is 128 pixels at full size, 4% of the width (about a
// degree and a half: my own tuning, from the hand's tremor and drift).
const int REACH = 8;

// A photo's frames (1.5.2) come a seventh of a second apart: the hand moves
// far less between them than over a night stack's second; 4 is 64 pixels at
// full size, and the search a quarter of the time.
const int PHOTO_REACH = 4;

// Coarse to fine: a sixteenth (all of the reach), then a quarter (about
// what that found). Full size is refined against the merge (Stack::add).
inline Shift align(const Pyramid &first, const Pyramid &next, int reach = REACH)
{
	Shift s = search(first.sixteenth, next.sixteenth, 0, 0, reach, 2, 1);
	Shift q = search(first.quarter, next.quarter, s.dx * 4, s.dy * 4, 3, 6, 2);
	q.dx *= 4;
	q.dy *= 4;
	return q;
}

// What a frame's alignment must reach to be merged: the mean difference
// at a quarter of the size, against the noise there (a frame of a scene
// that moved as a whole, or blurred by a hand moving fast, is left out).
inline bool lines_up(float err, float noise)
{
	return err <= 6.0f + 2.5f * noise;
}

// The noise, from a frame's differences to a reference (aligned by `s`), by
// brightness: the median of each (a pair's difference is the root of two
// noises: so much is taken off). `ref_y(i)` and `ref_u(i)` are the
// reference's Y and U at a plane's index i (Stack's merge so far, or the
// photo's first frame).
template <class RefY, class RefU>
void measure_noise(const RefY &ref_y, const RefU &ref_u, const uint8_t *yuv, unsigned w, unsigned h, Shift s, float noise[16], float &colour_noise)
{
	std::vector<uint32_t> hist(16 * 64, 0), chist(64, 0);
	const uint8_t *y = yuv, *u = yuv + size_t(w) * h;
	const unsigned cw2 = w / 2, ch2 = h / 2;
	for (unsigned py = 16; py + 16 < h; py += 3) {
		int sy = int(py) + s.dy;
		if (sy < 0 || sy >= int(h))
			continue;
		for (unsigned px = 16; px + 16 < w; px += 3) {
			int sx = int(px) + s.dx;
			if (sx < 0 || sx >= int(w))
				continue;
			int m = ref_y(size_t(py) * w + px);
			int d = std::min(63, std::abs(int(y[size_t(sy) * w + sx]) - m));
			hist[size_t(m >> 4) * 64 + d]++;
		}
	}
	for (unsigned py = 8; py + 8 < ch2; py += 3) {
		int sy = int(py) + s.dy / 2;
		if (sy < 0 || sy >= int(ch2))
			continue;
		for (unsigned px = 8; px + 8 < cw2; px += 3) {
			int sx = int(px) + s.dx / 2;
			if (sx < 0 || sx >= int(cw2))
				continue;
			int m = ref_u(size_t(py) * cw2 + px);
			chist[std::min(63, std::abs(int(u[size_t(sy) * cw2 + sx]) - m))]++;
		}
	}
	// The median of the differences' sizes, each whole level standing
	// for the half level either side of it.
	auto median = [](const uint32_t *bins, uint32_t total) {
		float half = float(total) / 2.0f, run = 0;
		for (int d = 0; d < 64; d++) {
			float in = float(bins[d]);
			if (run + in >= half && in > 0) {
				float from = d == 0 ? 0.0f : float(d) - 0.5f, width = d == 0 ? 0.5f : 1.0f;
				return from + width * (half - run) / in;
			}
			run += in;
		}
		return 63.0f;
	};
	// The median of |N(0, s√2)| is 0.6745 √2 s = 0.954 s.
	const float k = 1.0f / 0.954f;
	float known[16];
	bool have[16];
	for (int b = 0; b < 16; b++) {
		uint32_t total = 0;
		for (int d = 0; d < 64; d++)
			total += hist[size_t(b) * 64 + d];
		have[b] = total >= 200;
		known[b] = have[b] ? std::max(1.0f, median(&hist[size_t(b) * 64], total) * k) : 0.0f;
	}
	// Where a brightness was too rare to measure: the nearest that was.
	for (int b = 0; b < 16; b++) {
		if (have[b]) {
			noise[b] = known[b];
			continue;
		}
		float near = 2.0f;
		for (int r = 1; r < 16; r++) {
			if (b - r >= 0 && have[b - r]) {
				near = known[b - r];
				break;
			}
			if (b + r < 16 && have[b + r]) {
				near = known[b + r];
				break;
			}
		}
		noise[b] = near;
	}
	uint32_t ctotal = 0;
	for (uint32_t c : chist)
		ctotal += c;
	colour_noise = ctotal >= 200 ? std::max(1.0f, median(chist.data(), ctotal) * k) : 2.0f;
}

// The shift at full size: within two pixels of the quarter's, on 64
// patches of 32 x 32 against the reference (`ref_y(i)`, its Y at index i).
template <class RefY>
void refine_shift(const RefY &ref_y, const uint8_t *y, unsigned w, unsigned h, Shift &s)
{
	const unsigned P = 32, N = 8;
	if (w < 128 + 2 * P || h < 128 + 2 * P)
		return;
	int bdx = s.dx, bdy = s.dy;
	uint64_t best = ~0ull;
	for (int dy = s.dy - 2; dy <= s.dy + 2; dy++)
		for (int dx = s.dx - 2; dx <= s.dx + 2; dx++) {
			uint64_t sum = 0;
			for (unsigned gy = 0; gy < N; gy++)
				for (unsigned gx = 0; gx < N; gx++) {
					unsigned x0 = 64 + gx * (w - 128 - P) / (N - 1), y0 = 64 + gy * (h - 128 - P) / (N - 1);
					for (unsigned py = y0; py < y0 + P; py += 2) {
						int sy = int(py) + dy;
						if (sy < 0 || sy >= int(h))
							continue;
						for (unsigned px = x0; px < x0 + P; px += 2) {
							int sx = int(px) + dx;
							if (sx < 0 || sx >= int(w))
								continue;
							sum += unsigned(std::abs(int(y[size_t(sy) * w + sx]) - ref_y(size_t(py) * w + px)));
						}
					}
				}
			if (sum < best || (sum == best && std::abs(dx - s.dx) + std::abs(dy - s.dy) < std::abs(bdx - s.dx) + std::abs(bdy - s.dy))) {
				best = sum;
				bdx = dx;
				bdy = dy;
			}
		}
	s.dx = bdx;
	s.dy = bdy;
}

// The running sum.
//
// Weights are in quarters (0 to 4 a frame), so up to 63 frames fit the
// luma's 8-bit weight and 16-bit sum (still.h caps a stack at 32).
class Stack {
public:
	unsigned w = 0, h = 0;
	int frames = 0;
	// A frame's noise (its Y's standard deviation) at each sixteenth of the
	// brightness, and the colour's, once a second frame has shown it.
	float noise[16] = {};
	float colour_noise = 0;
	bool measured = false;

	Stack(unsigned w_, unsigned h_) : w(w_), h(h_)
	{
		ysum.assign(size_t(w) * h, 0);
		yw.assign(size_t(w) * h, 0);
		usum.assign(size_t(w / 2) * (h / 2), 0);
		vsum.assign(size_t(w / 2) * (h / 2), 0);
		cw.assign(size_t(w / 2) * (h / 2), 0);
		for (unsigned i = 1; i < 256; i++)
			recip[i] = (65536u + i / 2) / i;
	}

	// The first frame: the merge as it starts, and what the rest line up to.
	void first(const uint8_t *yuv)
	{
		const uint8_t *y = yuv, *u = yuv + size_t(w) * h, *v = u + size_t(w / 2) * (h / 2);
		for (size_t i = 0; i < size_t(w) * h; i++) {
			ysum[i] = uint16_t(4 * y[i]);
			yw[i] = 4;
		}
		for (size_t i = 0; i < size_t(w / 2) * (h / 2); i++) {
			usum[i] = uint16_t(4 * u[i]);
			vsum[i] = uint16_t(4 * v[i]);
			cw[i] = 4;
		}
		reference = pyramid(y, w, h);
		frames = 1;
	}

	// A frame merged in, if it lines up: false if it was left out.
	bool add(const uint8_t *yuv, unsigned threads, Shift *found = nullptr)
	{
		const uint8_t *y = yuv;
		Pyramid p = pyramid(y, w, h);
		Shift s = align(reference, p);
		// Before the noise is known, as much as a gain of 10 gives (about 8).
		float level_noise = measured ? (noise[8] > 0 ? noise[8] : 2.0f) : 8.0f;
		if (!lines_up(s.err, level_noise / 2.0f)) {
			if (found)
				*found = s;
			return false;
		}
		// The noise from the first frame that lines up (one that does not
		// would make every difference look like noise).
		if (!measured)
			measure(yuv, s);
		refine(y, s);
		if (found)
			*found = s;
		accumulate(yuv, s, threads);
		frames++;
		return true;
	}

	// The merge as it stands: each pixel's weighted mean, packed YUV 4:2:0.
	void result(uint8_t *out) const
	{
		for (size_t i = 0; i < size_t(w) * h; i++)
			out[i] = mean(ysum[i], yw[i]);
		uint8_t *u = out + size_t(w) * h, *v = u + size_t(w / 2) * (h / 2);
		for (size_t i = 0; i < size_t(w / 2) * (h / 2); i++) {
			u[i] = mean(usum[i], cw[i]);
			v[i] = mean(vsum[i], cw[i]);
		}
	}

	// The noise left after the merge, at mid brightness (for the finish's
	// filter): a frame's, over the root of the frames.
	float residual() const
	{
		float n = noise[8] > 0 ? noise[8] : 2.0f;
		return n / std::sqrt(float(std::max(1, frames)));
	}

private:
	std::vector<uint16_t> ysum, usum, vsum;
	std::vector<uint8_t> yw, cw;
	uint32_t recip[256] = {};
	Pyramid reference;

	uint8_t mean(uint32_t sum, uint32_t weight) const
	{
		if (weight == 0)
			return 0;
		return uint8_t(std::min<uint32_t>(255, ((sum + weight / 2) * recip[weight]) >> 16));
	}

	// The second frame shows the noise (measure_noise), against the merge
	// so far: the first frame alone then.
	void measure(const uint8_t *yuv, Shift s)
	{
		measure_noise([&](size_t i) { return int(mean(ysum[i], yw[i])); }, [&](size_t i) { return int(mean(usum[i], cw[i])); }, yuv, w, h, s, noise, colour_noise);
		measured = true;
	}

	// The shift at full size, against the merge so far (refine_shift).
	void refine(const uint8_t *y, Shift &s) const
	{
		refine_shift([&](size_t i) { return int(mean(ysum[i], yw[i])); }, y, w, h, s);
	}

	// Each pixel weighed against the merge so far and added in.
	void accumulate(const uint8_t *yuv, Shift s, unsigned threads)
	{
		// The weight of a difference at each brightness: in full within
		// three of the difference's noise (a frame's against a merge of n:
		// the root of 1 + 1/n of a frame's), none past six.
		uint8_t table[16][256];
		const float spread = std::sqrt(1.0f + 1.0f / float(frames));
		for (int b = 0; b < 16; b++) {
			float t = std::max(2.0f, 3.0f * noise[b] * spread);
			for (int d = 0; d < 256; d++)
				table[b][d] = uint8_t(d <= t ? 4 : d >= 2 * t ? 0 : std::lround(4.0f * (2.0f * t - float(d)) / t));
		}
		uint8_t ctable[256];
		{
			float t = std::max(3.0f, 3.0f * colour_noise * spread);
			for (int d = 0; d < 256; d++)
				ctable[d] = uint8_t(d <= t ? 4 : d >= 2 * t ? 0 : std::lround(4.0f * (2.0f * t - float(d)) / t));
		}
		const uint8_t *y = yuv, *u = yuv + size_t(w) * h, *v = u + size_t(w / 2) * (h / 2);
		const unsigned cw2 = w / 2, ch2 = h / 2;
		// Chroma moves half as far (an odd shift rounds toward none).
		const int cdx = s.dx / 2, cdy = s.dy / 2;
		in_bands(ch2, threads, [&](unsigned c0, unsigned c1) {
			for (unsigned cy = c0; cy < c1; cy++) {
				for (unsigned cx = 0; cx < cw2; cx++) {
					uint8_t least = 4;
					for (unsigned k = 0; k < 4; k++) {
						unsigned px = 2 * cx + (k & 1), py = 2 * cy + (k >> 1);
						int sx = int(px) + s.dx, sy = int(py) + s.dy;
						if (sx < 0 || sy < 0 || sx >= int(w) || sy >= int(h)) {
							least = 0;
							continue;
						}
						size_t i = size_t(py) * w + px;
						uint8_t m = mean(ysum[i], yw[i]);
						uint8_t n = y[size_t(sy) * w + sx];
						uint8_t wt = table[m >> 4][std::abs(int(n) - int(m))];
						ysum[i] = uint16_t(ysum[i] + wt * n);
						yw[i] = uint8_t(yw[i] + wt);
						least = std::min(least, wt);
					}
					int sx = int(cx) + cdx, sy = int(cy) + cdy;
					if (least == 0 || sx < 0 || sy < 0 || sx >= int(cw2) || sy >= int(ch2))
						continue;
					size_t i = size_t(cy) * cw2 + cx, j = size_t(sy) * cw2 + sx;
					uint8_t nu = u[j], nv = v[j];
					uint8_t mu = mean(usum[i], cw[i]), mv = mean(vsum[i], cw[i]);
					uint8_t wt = std::min({ least, ctable[std::abs(int(nu) - int(mu))], ctable[std::abs(int(nv) - int(mv))] });
					usum[i] = uint16_t(usum[i] + wt * nu);
					vsum[i] = uint16_t(vsum[i] + wt * nv);
					cw[i] = uint8_t(cw[i] + wt);
				}
			}
		});
	}
};

// ------------------------------------------------------- noise, measured ---

// A photo's own noise (Y's standard deviation), when no merge measured it:
// Immerkær's 3 x 3 operator (two second differences, which a smooth or
// straight-edged picture gives none of), taken on every third pixel of every
// fourth row, its quarter-way point (the flattest quarter of the picture
// speaks for its noise; texture and edges only raise the rest). For noise
// alone the operator's spread is 6 sigma, and a quarter of |N(0, 6 s)| lies
// under 0.3186 x 6 s.
inline float estimate_noise(const uint8_t *y, unsigned w, unsigned h)
{
	if (w < 16 || h < 16)
		return 1.0f;
	std::vector<uint32_t> hist(256, 0);
	uint32_t total = 0;
	for (unsigned py = 4; py + 4 < h; py += 4) {
		const uint8_t *a = y + size_t(py - 1) * w, *b = a + w, *c = b + w;
		for (unsigned px = 4; px + 4 < w; px += 3) {
			int l = a[px - 1] - 2 * a[px] + a[px + 1] - 2 * b[px - 1] + 4 * b[px] - 2 * b[px + 1] + c[px - 1] - 2 * c[px] + c[px + 1];
			// Clipped pixels say nothing of the noise.
			if (b[px] < 4 || b[px] > 251)
				continue;
			hist[std::min(255, std::abs(l))]++;
			total++;
		}
	}
	if (total < 1000)
		return 1.0f;
	uint32_t run = 0, quarter = total / 4;
	int at = 0;
	for (; at < 255; at++) {
		if (run + hist[size_t(at)] >= quarter)
			break;
		run += hist[size_t(at)];
	}
	// Within the level, as far as the quarter falls in it.
	float in = hist[size_t(at)] ? float(quarter - run) / float(hist[size_t(at)]) : 0.0f;
	float q = std::max(0.0f, float(at) - 0.5f + in);
	return std::clamp(q / (0.3186f * 6.0f), 0.5f, 16.0f);
}

// ---------------------------------------------------------- photo merge ---
//
// Every photo (1.5.2): a few frames taken back to back (three, a seventh of
// a second apart), held in memory while the preview runs again, and merged
// into the first as the photo is written, as a phone does. All of them are
// there at once, so unlike a night stack no sums are kept: the first frame
// is the reference and takes the merge in place. Three full-size frames are
// 36 MB, under a stack's 46.
//
// - Alignment as a stack's: one shift for the picture, the search a quarter
//   as wide (PHOTO_REACH), refined at full size against the first; the
//   noise measured the same way, from the second frame's differences.
// - Each pixel the weighed mean of the frames, the first counting in full.
//   A frame's weight at a pixel comes from its differences to the first
//   over the 3 x 3 around it, not the pixel's alone: noise differs there by
//   about 1.13 times the root of two noises on average and at random; what
//   moved, or fine texture a hand's half pixel out, differs more and
//   steadily, and is left to the first frame. So detail is never softened
//   by averaging pictures that do not quite match. Full weight within 1.5
//   times what noise gives, none past 2.5 times.
// - Colour: as the luma said of its four pixels, and its own difference.

struct Merged {
	// Frames that lined up and were merged, the first included.
	int frames = 1;
	// A frame's noise at mid brightness, and what is left after the merge.
	float noise = 0, residual = 0;
};

inline Merged merge_frames(uint8_t *first, const std::vector<const uint8_t *> &more, unsigned w, unsigned h, unsigned threads)
{
	Merged out;
	if (more.empty() || w < 64 || h < 64)
		return out;
	const size_t luma = size_t(w) * h;
	const unsigned cw2 = w / 2, ch2 = h / 2;
	uint8_t *y0 = first, *u0 = first + luma, *v0 = u0 + size_t(cw2) * ch2;
	// Every frame's brightness made small, and each found against the
	// first, all at once (a core each: they need nothing of one another).
	std::vector<Pyramid> small(more.size() + 1);
	std::vector<Shift> found(more.size());
	{
		std::vector<std::thread> pool;
		for (size_t k = 0; k <= more.size(); k++)
			pool.emplace_back([&, k] { small[k] = pyramid(k == 0 ? y0 : more[k - 1], w, h); });
		for (std::thread &t : pool)
			t.join();
		pool.clear();
		for (size_t k = 0; k < more.size(); k++)
			pool.emplace_back([&, k] { found[k] = align(small[0], small[k + 1], PHOTO_REACH); });
		for (std::thread &t : pool)
			t.join();
	}
	float noise[16] = {};
	float colour_noise = 2.0f;
	bool measured = false;
	struct Use {
		const uint8_t *yuv;
		Shift s;
	};
	std::vector<Use> use;
	for (size_t k = 0; k < more.size(); k++) {
		const uint8_t *f = more[k];
		Shift s = found[k];
		float level = measured ? (noise[8] > 0 ? noise[8] : 2.0f) : 8.0f;
		if (!lines_up(s.err, level / 2.0f))
			continue;
		if (!measured) {
			measure_noise([&](size_t i) { return int(y0[i]); }, [&](size_t i) { return int(u0[i]); }, f, w, h, s, noise, colour_noise);
			measured = true;
		}
		refine_shift([&](size_t i) { return int(y0[i]); }, f, w, h, s);
		use.push_back({ f, s });
	}
	// A frame-to-frame difference counts what moved and texture a hand's
	// fraction of a pixel out as noise too: in a detailed scene the noise
	// measured that way runs high, and so would the merge's leniency (what
	// it averages, it softens). Never above one and a half times the photo's own.
	if (measured) {
		const float cap = std::max(1.0f, 1.5f * estimate_noise(y0, w, h));
		for (float &v : noise)
			v = std::min(v, cap);
		colour_noise = std::min(colour_noise, cap);
	}
	out.noise = out.residual = measured ? noise[8] : 0.0f;
	if (use.empty())
		return out;
	const int n = int(use.size());
	// The weight (0 to 4) of a 3 x 3 sum of differences s at each
	// brightness: 4 up to lo, then down to none at top.
	int lo[16], top[16], scale[16];
	for (int b = 0; b < 16; b++) {
		float unit = 9.0f * 1.128f * 1.4142f * std::max(0.7f, noise[b]);
		lo[b] = int(1.5f * unit + 4.5f);
		top[b] = std::max(lo[b] + 1, int(2.5f * unit + 9.0f));
		scale[b] = (4 << 16) / (top[b] - lo[b]);
	}
	uint8_t ctable[256];
	{
		float t = std::max(3.0f, 3.0f * colour_noise * 1.4142f);
		for (int d = 0; d < 256; d++)
			ctable[d] = uint8_t(d <= t ? 4 : d >= 2 * t ? 0 : std::lround(4.0f * (2.0f * t - float(d)) / t));
	}
	uint32_t recip[64];
	recip[0] = 0;
	for (unsigned i = 1; i < 64; i++)
		recip[i] = (65536u + i / 2) / i;
	// The bands' edge rows of the first frame, kept as they are: a band
	// reads its neighbours' nearest rows, which they may have merged by then.
	threads = std::max(1u, std::min(threads, ch2));
	const unsigned step = (ch2 + threads - 1) / threads;
	std::vector<std::vector<uint8_t>> kept(static_cast<size_t>(h));
	for (unsigned t = 1; t < threads; t++) {
		unsigned r = 2 * t * step;
		if (r >= h)
			break;
		kept[r - 1].assign(y0 + size_t(r - 1) * w, y0 + size_t(r) * w);
		kept[r].assign(y0 + size_t(r) * w, y0 + size_t(r + 1) * w);
	}
	std::vector<uint64_t> weighed(size_t(threads) * size_t(n), 0);
	in_bands(ch2, threads, [&](unsigned c0, unsigned c1) {
		const unsigned r0 = 2 * c0, r1 = 2 * c1, band = c0 / step;
		// A row of the first frame: its own (not yet merged when read), else
		// as kept.
		auto ref_row = [&](unsigned r) -> const uint8_t * {
			if ((r < r0 || r >= r1) && !kept[r].empty())
				return kept[r].data();
			return y0 + size_t(r) * w;
		};
		// For each frame, the differences summed across for three rows (the
		// one before, this one, the next: by row number mod 3), and the
		// weights of this row and of the even row above it.
		std::vector<uint16_t> across(size_t(n) * 3 * w);
		std::vector<uint8_t> even(size_t(n) * w), wt(size_t(n) * w), d(w);
		auto slot = [&](int k, unsigned r) { return &across[(size_t(k) * 3 + r % 3) * w]; };
		auto differ = [&](int k, unsigned r, uint16_t *sums) {
			const Use &f = use[size_t(k)];
			const uint8_t *ref = ref_row(r);
			int sy = int(r) + f.s.dy;
			if (sy < 0 || sy >= int(h)) {
				std::fill(sums, sums + w, uint16_t(765));
				return;
			}
			const uint8_t *row = f.yuv + size_t(sy) * w;
			for (unsigned x = 0; x < w; x++) {
				int sx = int(x) + f.s.dx;
				d[x] = (sx < 0 || sx >= int(w)) ? 255 : uint8_t(std::abs(int(row[sx]) - int(ref[x])));
			}
			sums[0] = uint16_t(2 * d[0] + d[1]);
			for (unsigned x = 1; x + 1 < w; x++)
				sums[x] = uint16_t(d[x - 1] + d[x] + d[x + 1]);
			sums[w - 1] = uint16_t(d[w - 2] + 2 * d[w - 1]);
		};
		// The row above the band's first (itself at the top of the picture).
		for (int k = 0; k < n; k++) {
			differ(k, r0 == 0 ? 0 : r0 - 1, slot(k, r0 + 2));
			differ(k, r0, slot(k, r0));
		}
		for (unsigned py = r0; py < r1; py++) {
			for (int k = 0; k < n; k++)
				differ(k, std::min(py + 1, h - 1), slot(k, py + 1));
			uint8_t *y = y0 + size_t(py) * w;
			for (unsigned x = 0; x < w; x++) {
				const int b = y[x] >> 4;
				uint32_t sum = 4u * y[x], weight = 4;
				for (int k = 0; k < n; k++) {
					const Use &f = use[size_t(k)];
					int s = int(slot(k, py + 2)[x]) + int(slot(k, py)[x]) + int(slot(k, py + 1)[x]);
					int v = s <= lo[b] ? 4 : std::clamp(((top[b] - s) * scale[b]) >> 16, 0, 4);
					int sx = int(x) + f.s.dx, sy = int(py) + f.s.dy;
					if (sx < 0 || sy < 0 || sx >= int(w) || sy >= int(h))
						v = 0;
					wt[size_t(k) * w + x] = uint8_t(v);
					if (v) {
						sum += uint32_t(v) * f.yuv[size_t(sy) * w + sx];
						weight += uint32_t(v);
					}
				}
				y[x] = uint8_t(std::min<uint32_t>(255, ((sum + weight / 2) * recip[weight]) >> 16));
			}
			for (int k = 0; k < n; k++) {
				uint64_t total = 0;
				for (unsigned x = 0; x < w; x++)
					total += wt[size_t(k) * w + x];
				weighed[size_t(band) * size_t(n) + size_t(k)] += total;
			}
			if (py % 2 == 0) {
				even.swap(wt);
				continue;
			}
			// The colour of these two rows, as their luma said.
			const unsigned cy = py / 2;
			uint8_t *u = u0 + size_t(cy) * cw2, *v = v0 + size_t(cy) * cw2;
			for (unsigned cx = 0; cx < cw2; cx++) {
				uint32_t su = 4u * u[cx], sv = 4u * v[cx], weight = 4;
				for (int k = 0; k < n; k++) {
					const Use &f = use[size_t(k)];
					const size_t at = size_t(k) * w + 2 * cx;
					uint8_t least = std::min({ even[at], even[at + 1], wt[at], wt[at + 1] });
					int sx = int(cx) + f.s.dx / 2, sy = int(cy) + f.s.dy / 2;
					if (least == 0 || sx < 0 || sy < 0 || sx >= int(cw2) || sy >= int(ch2))
						continue;
					const uint8_t *fu = f.yuv + luma, *fv = fu + size_t(cw2) * ch2;
					uint8_t nu = fu[size_t(sy) * cw2 + sx], nv = fv[size_t(sy) * cw2 + sx];
					uint8_t c = std::min({ least, ctable[std::abs(int(nu) - int(u[cx]))], ctable[std::abs(int(nv) - int(v[cx]))] });
					su += uint32_t(c) * nu;
					sv += uint32_t(c) * nv;
					weight += c;
				}
				u[cx] = uint8_t(std::min<uint32_t>(255, ((su + weight / 2) * recip[weight]) >> 16));
				v[cx] = uint8_t(std::min<uint32_t>(255, ((sv + weight / 2) * recip[weight]) >> 16));
			}
		}
	});
	// What is left: a frame's noise over the root of the frames counted,
	// each at its mean weight.
	float counted = 1.0f;
	for (int k = 0; k < n; k++) {
		uint64_t total = 0;
		for (unsigned t = 0; t < threads; t++)
			total += weighed[size_t(t) * size_t(n) + size_t(k)];
		counted += float(double(total) / (4.0 * double(luma)));
	}
	out.frames = 1 + n;
	out.residual = out.noise / std::sqrt(counted);
	return out;
}

// --------------------------------------------------------------- finish ---

// How a merged (or single) photo is finished.
struct Finish {
	// Stops of brightness the frames could not reach (night) or were kept
	// under to save the highlights (HDR, a guarded photo), given back to
	// the mid-tones; the highlights rolled into white on a shoulder.
	float lift = 0;
	// The shadows lifted a little more (0 none, 0.5 a lot).
	float toe = 0;
	// How far near-grey colour is pulled toward neutral (0 to 1).
	float neutral = 0;
	// The noise left (Y's standard deviation at mid brightness): the
	// filter's reach; 0 for no filter.
	float noise = 0;
	// 1.5.2: the noise known to be left (a merge's), for the sharpening and
	// the local tone; 0: measured from the photo itself.
	float grain = 0;
	// The local tone (0 none, 1 in full): the shadows lifted where they are
	// dark, a lift's highlights held where they are bright, a face brought
	// toward a face's level (gain_map).
	float local = 0;
	// Sharpening on the luma (0 none, 1 in full: sharpen).
	float crisp = 0;
	// A face in the photo (0 to 1 each way, the picture's way up).
	bool face = false;
	float face_box[4] = { 0, 0, 0, 0 };

	bool any() const { return lift > 0.01f || toe > 0.01f || neutral > 0.01f || noise > 0.3f || local > 0.01f || crisp > 0.01f; }
};

// The tone curve, as a table on Y: the lift in linear light (the encoding's
// 2.2 undone), an extended Reinhard shoulder that takes the old white to
// white, back to 2.2, then the toe.
inline void tone_table(float lift, float toe, uint8_t out[256])
{
	const float gain = std::pow(2.0f, std::max(0.0f, lift));
	const float white2 = gain * gain;
	for (int i = 0; i < 256; i++) {
		float lin = std::pow(float(i) / 255.0f, 2.2f);
		float x = lin * gain;
		float t = x * (1.0f + x / white2) / (1.0f + x);
		// The shoulder is only for what was lifted: none, none.
		if (gain <= 1.0001f)
			t = lin;
		float s = std::pow(std::clamp(t, 0.0f, 1.0f), 1.0f / 2.2f);
		s += toe * 1.5f * s * std::pow(1.0f - s, 4.0f);
		out[i] = uint8_t(std::clamp(std::lround(s * 255.0f), 0L, 255L));
	}
}

// The mean colour of what is near grey (chroma within 24 of neutral, not
// too dark or bright to say): (U, V) less 128.
inline void cast_of(const uint8_t *yuv, unsigned w, unsigned h, float &cu, float &cv)
{
	const uint8_t *y = yuv, *u = yuv + size_t(w) * h, *v = u + size_t(w / 2) * (h / 2);
	int64_t su = 0, sv = 0, n = 0;
	for (unsigned cy = 0; cy < h / 2; cy += 3)
		for (unsigned cx = 0; cx < w / 2; cx += 3) {
			int l = y[size_t(2 * cy) * w + 2 * cx];
			int du = int(u[size_t(cy) * (w / 2) + cx]) - 128, dv = int(v[size_t(cy) * (w / 2) + cx]) - 128;
			if (l < 32 || l > 224 || std::abs(du) > 24 || std::abs(dv) > 24)
				continue;
			su += du;
			sv += dv;
			n++;
		}
	cu = n > 100 ? float(su) / float(n) : 0.0f;
	cv = n > 100 ? float(sv) / float(n) : 0.0f;
}

// ------------------------------------------------------------- sharpening ---
//
// 1.5.2: the photo's own sharpening, in place of most of the ISP's (which
// sharpens every frame before the merge, its noise too, and rings at hard
// edges). On the luma, after the merge and the noise filter, before the tone:
//
// - The detail: the luma less its 5 x 5 Gaussian blur (sigma about one
//   pixel, the IMX219's lens and demosaic softness).
// - Only where there is texture: by the 3 x 3's range against what noise
//   alone gives (about three sigma), so flat sky and low-texture areas are
//   left alone.
// - Cored where the texture is faint: a difference within about twice the
//   noise is mostly noise there and is left alone (d x d^2 / (d^2 + t^2));
//   where the texture is plain (an edge), not, as the edge's own small
//   differences beside its middle are what make it crisp.
// - Skin held back: where the colour is a skin tone (Cb 77 to 127, Cr 133
//   to 173 by Chai and Ngan, softened at the edges), a fifth as much.
// - No halos: each result kept within the 3 x 3's darkest and brightest,
//   give or take a level or two (a little more on a strong edge).
// - Less as the noise rises: the amount falls to 0.35 by a noise of 7.

// A skin tone's share (0 to 256) at a chroma value: within lo to hi in
// full, falling off over 8 levels either side.
inline int skin_share(int c, int lo, int hi)
{
	if (c >= lo && c <= hi)
		return 256;
	int out = c < lo ? lo - c : c - hi;
	return out >= 8 ? 0 : 256 - out * 32;
}

inline void sharpen(uint8_t *yuv, unsigned w, unsigned h, float amount, float noise, unsigned threads)
{
	if (amount <= 0.01f || w < 16 || h < 16)
		return;
	uint8_t *y = yuv;
	const uint8_t *u = yuv + size_t(w) * h, *v = u + size_t(w / 2) * (h / 2);
	const unsigned cw2 = w / 2, ch2 = h / 2;
	const float sigma = std::max(0.6f, noise);
	// At 1, half as much again of the detail (less as the noise rises).
	const float a = 1.5f * amount * std::clamp(1.25f - sigma / 8.0f, 0.35f, 1.0f);
	// The cored detail's gain, in sixteenths, for each difference -255 to 255.
	const float t = 2.0f * sigma;
	// (and uncored, for where the texture is plain: there a small
	// difference is the edge's own, not noise).
	std::vector<int16_t> core(511), plain(511);
	for (int d = -255; d <= 255; d++) {
		float f = float(d);
		core[size_t(d + 255)] = int16_t(std::lround(16.0f * a * f * f * f / (f * f + t * t)));
		plain[size_t(d + 255)] = int16_t(std::lround(16.0f * a * f));
	}
	// Texture's share (0 to 256) by the 3 x 3's range: none up to what
	// noise alone gives, in full from twice that and a little.
	uint16_t texture[256], margin[256];
	{
		const float flat = 3.0f * sigma + 2.0f, full = 2.0f * flat + 6.0f;
		for (int r = 0; r < 256; r++) {
			float s = std::clamp((float(r) - flat) / (full - flat), 0.0f, 1.0f);
			texture[r] = uint16_t(std::lround(256.0f * s * s * (3.0f - 2.0f * s)));
			margin[r] = uint16_t(std::min(4, 1 + r / 40));
		}
	}
	// Skin's sharpening (0 to 256): a quarter where the colour is skin's.
	uint16_t skin_u[256], skin_v[256];
	for (int c = 0; c < 256; c++) {
		skin_u[c] = uint16_t(skin_share(c, 77, 127));
		skin_v[c] = uint16_t(skin_share(c, 133, 173));
	}
	std::vector<uint8_t> src(y, y + size_t(w) * h);
	in_bands(ch2, threads, [&](unsigned c0, unsigned c1) {
		std::vector<uint16_t> down(w);
		std::vector<uint8_t> lo(w), hi(w);
		for (unsigned py = 2 * c0; py < 2 * c1; py++) {
			if (py < 2 || py + 2 >= h)
				continue;
			const uint8_t *r[5];
			for (int k = 0; k < 5; k++)
				r[k] = &src[size_t(py + unsigned(k) - 2) * w];
			// Down the five rows (1 4 6 4 1), and the three rows' least and most.
			for (unsigned x = 0; x < w; x++) {
				down[x] = uint16_t(r[0][x] + 4 * r[1][x] + 6 * r[2][x] + 4 * r[3][x] + r[4][x]);
				lo[x] = std::min({ r[1][x], r[2][x], r[3][x] });
				hi[x] = std::max({ r[1][x], r[2][x], r[3][x] });
			}
			const uint8_t *cu = u + size_t(py / 2) * cw2, *cv = v + size_t(py / 2) * cw2;
			uint8_t *out = y + size_t(py) * w;
			for (unsigned x = 2; x + 2 < w; x++) {
				int blur = (down[x - 2] + 4 * down[x - 1] + 6 * down[x] + 4 * down[x + 1] + down[x + 2] + 128) >> 8;
				int s = r[2][x];
				int least = std::min({ lo[x - 1], lo[x], lo[x + 1] }), most = std::max({ hi[x - 1], hi[x], hi[x + 1] });
				int range = most - least;
				const int share = texture[range];
				if (share == 0)
					continue;
				// Cored where the texture is faint, less so as it is plain.
				const size_t d = size_t(s - blur + 255);
				const int detail = (core[d] * (256 - share) + plain[d] * share) >> 8;
				const int skin = (skin_u[cu[x / 2]] * skin_v[cv[x / 2]]) >> 8;
				const int amount = (share * (256 - ((skin * 13) >> 4))) >> 8;
				int add = (detail * amount) >> 12;
				int m = margin[range];
				out[x] = uint8_t(std::clamp(s + add, std::max(0, least - m), std::min(255, most + m)));
			}
		}
	});
}

// ------------------------------------------------------------ local tone ---
//
// 1.5.2: the exposure set region by region, a little, as a phone's local
// tone mapping does, so a face in shade and the sky behind it are both
// right, without the flat look: only the large-scale brightness moves, the
// detail and contrast within each region are kept as they are.
//
// - The base: the luma's means over blocks (16 x 16 at full size, about
//   200 across), in stops of linear light, smoothed with a bilateral filter
//   (about a thirtieth of the width across, but only among blocks within a
//   stop or so of each other), so the base follows the scene's edges.
// - Its stops (local_stops): only real shadows lifted (darker than about
//   75 as encoded), 0.3 of the way and a stop at most (less as the
//   noise rises), the blacks left black; where a lift was asked (the frames
//   kept under to save the sky), the brightest regions given back a third
//   less of it; a face brought half way to a face's usual level
//   (FACE_TONE), a stop up or half a stop down at most.
// - Onto every pixel: the blocks' gains fitted to their brightness over
//   each 3 x 3 of blocks as a line, gain = a I + b (He and Sun's fast
//   guided filter), the lines spread bilinearly across the picture and
//   each pixel taking the gain its own brightness gives. Where blocks
//   straddle an edge, the dark side takes the dark side's gain and the
//   bright side the bright side's: no halo. A pixel lifted toward white is
//   lifted less, so nothing lifted clips.

// A face well exposed: its mean brightness, 0 to 1 as encoded (as the
// preview's face metering aims for).
const float FACE_TONE = 0.47f;

inline float smooth_step(float a, float b, float x)
{
	float t = std::clamp((x - a) / (b - a), 0.0f, 1.0f);
	return t * t * (3.0f - 2.0f * t);
}

// The local exposure (stops) of a region whose base is `base` (stops of
// linear light: 0 white, -2.5 about mid-grey), for a finish's strength,
// lift and the noise.
inline float local_stops(float base, float strength, float lift, float noise)
{
	// The shadows, as they will be after the lift: only what is darker than
	// about 75 (as encoded), 0.3 of the way toward it, a stop at most.
	const float target = -3.9f;
	float after = base + lift;
	float most = std::clamp(1.3f - noise / 6.0f, 0.5f, 1.0f);
	float up = std::min(most, 0.3f * std::max(0.0f, target - after));
	// The blacks left black (no grey veil, no noise raised out of them).
	up *= smooth_step(-10.5f, -8.5f, base);
	// A lift's highlights held where the lift would take them past the
	// shoulder (as kept, brighter than about 160): a third of it back.
	float down = 0.35f * lift * smooth_step(-1.6f, -0.4f, base);
	return strength * (up - down);
}

// The local tone's gains, at a block's size: each block's line (a, b), so
// a pixel of brightness I (0 to 1) is multiplied by a I + b, within lo to hi.
struct GainMap {
	unsigned block = 16, gw = 0, gh = 0;
	std::vector<float> a, b;
	float lo = 1, hi = 1;
};

inline GainMap gain_map(const uint8_t *y, unsigned w, unsigned h, const Finish &how, float noise, const uint8_t table[256], unsigned threads)
{
	GainMap g;
	g.block = w >= 2400 ? 16 : w >= 1200 ? 8 : 4;
	const unsigned k = g.block;
	g.gw = std::max(1u, w / k);
	g.gh = std::max(1u, h / k);
	const unsigned gw = g.gw, gh = g.gh;
	const size_t cells = size_t(gw) * gh;
	// The blocks' means (0 to 1, as encoded).
	std::vector<float> mean(cells);
	in_bands(gh, threads, [&](unsigned b0, unsigned b1) {
		for (unsigned by = b0; by < b1; by++)
			for (unsigned bx = 0; bx < gw; bx++) {
				uint32_t sum = 0;
				for (unsigned dy = 0; dy < k && by * k + dy < h; dy++) {
					const uint8_t *row = y + size_t(by * k + dy) * w + size_t(bx) * k;
					for (unsigned dx = 0; dx < k; dx++)
						sum += row[dx];
				}
				mean[size_t(by) * gw + bx] = float(sum) / float(k * k * 255);
			}
	});
	// In stops of linear light.
	std::vector<float> stops(cells), base(cells);
	for (size_t i = 0; i < cells; i++)
		stops[i] = 2.2f * std::log2(std::max(mean[i], 1.0f / 1024.0f));
	// The base: bilateral, about a thirtieth of the width across, among
	// blocks within about a stop.
	{
		const float sigma_s = std::max(1.5f, float(gw) / 32.0f);
		const int reach = int(std::ceil(2.0f * sigma_s));
		const int stride = std::max(1, reach / 6);
		std::vector<float> near(size_t(2 * reach + 1) * size_t(2 * reach + 1));
		for (int dy = -reach; dy <= reach; dy++)
			for (int dx = -reach; dx <= reach; dx++)
				near[size_t(dy + reach) * size_t(2 * reach + 1) + size_t(dx + reach)] = std::exp(-float(dx * dx + dy * dy) / (2.0f * sigma_s * sigma_s));
		// By the difference in stops, in sixteenths, up to 8 stops.
		float alike[129];
		for (int i = 0; i <= 128; i++) {
			float d = float(i) / 16.0f;
			alike[i] = std::exp(-d * d / (2.0f * 0.9f * 0.9f));
		}
		in_bands(gh, threads, [&](unsigned b0, unsigned b1) {
			for (int by = int(b0); by < int(b1); by++)
				for (int bx = 0; bx < int(gw); bx++) {
					float c = stops[size_t(by) * gw + size_t(bx)], sum = 0, weight = 0;
					for (int dy = -reach; dy <= reach; dy += stride) {
						int yy = by + dy;
						if (yy < 0 || yy >= int(gh))
							continue;
						for (int dx = -reach; dx <= reach; dx += stride) {
							int xx = bx + dx;
							if (xx < 0 || xx >= int(gw))
								continue;
							float s = stops[size_t(yy) * gw + size_t(xx)];
							int d = std::min(128, int(std::fabs(s - c) * 16.0f));
							float wgt = near[size_t(dy + reach) * size_t(2 * reach + 1) + size_t(dx + reach)] * alike[d];
							sum += wgt * s;
							weight += wgt;
						}
					}
					base[size_t(by) * gw + size_t(bx)] = weight > 0 ? sum / weight : c;
				}
		});
	}
	// Each block's exposure, in stops.
	std::vector<float> ev(cells);
	for (size_t i = 0; i < cells; i++)
		ev[i] = local_stops(base[i], how.local, how.lift, noise);
	// A face: its blocks' level as the photo would be, brought half way to
	// a face's (a stop up at most, half a stop down), over the face and a
	// little around it, fading.
	if (how.face) {
		float x0 = std::min(how.face_box[0], how.face_box[2]), x1 = std::max(how.face_box[0], how.face_box[2]);
		float y0 = std::min(how.face_box[1], how.face_box[3]), y1 = std::max(how.face_box[1], how.face_box[3]);
		x0 = std::clamp(x0, 0.0f, 1.0f), x1 = std::clamp(x1, 0.0f, 1.0f), y0 = std::clamp(y0, 0.0f, 1.0f), y1 = std::clamp(y1, 0.0f, 1.0f);
		float cx = (x0 + x1) / 2.0f * float(gw), cy = (y0 + y1) / 2.0f * float(gh);
		float rx = std::max(1.0f, (x1 - x0) / 2.0f * float(gw)), ry = std::max(1.0f, (y1 - y0) / 2.0f * float(gh));
		double level = 0, face_stops = 0;
		int counted = 0;
		// Its middle (the box's inner 60%), as the photo would have it.
		for (unsigned by = 0; by < gh; by++)
			for (unsigned bx = 0; bx < gw; bx++) {
				float ex = (float(bx) + 0.5f - cx) / rx, ey = (float(by) + 0.5f - cy) / ry;
				if (std::fabs(ex) > 0.6f || std::fabs(ey) > 0.6f)
					continue;
				size_t i = size_t(by) * gw + bx;
				float lit = std::clamp(mean[i] * std::pow(2.0f, ev[i] / 2.2f), 0.0f, 1.0f);
				level += table[int(lit * 255.0f + 0.5f)] / 255.0;
				face_stops += base[i];
				counted++;
			}
		if (counted > 0) {
			float seen = std::max(float(level / counted), 0.02f);
			float wrong = 2.2f * std::log2(FACE_TONE / seen);
			float extra = std::fabs(wrong) < 0.25f ? 0.0f : std::clamp(0.5f * wrong, -0.5f, 1.0f) * std::max(0.6f, std::min(1.0f, how.local));
			// Only on what is about as bright as the face (within a stop or
			// so): the wall behind it is not lifted with it.
			const float like = float(face_stops / counted);
			if (extra != 0.0f)
				for (unsigned by = 0; by < gh; by++)
					for (unsigned bx = 0; bx < gw; bx++) {
						float ex = (float(bx) + 0.5f - cx) / (1.4f * rx), ey = (float(by) + 0.5f - cy) / (1.4f * ry);
						float r2 = ex * ex + ey * ey;
						size_t i = size_t(by) * gw + bx;
						float d = base[i] - like;
						ev[i] += extra * (r2 <= 1.0f ? 1.0f : std::exp(-(r2 - 1.0f) * 2.0f)) * std::exp(-d * d / (2.0f * 1.2f * 1.2f));
					}
		}
	}
	// As a multiplier on the encoded brightness (linear light to the 1/2.2).
	std::vector<float> p(cells);
	g.lo = 1e9f;
	g.hi = -1e9f;
	for (size_t i = 0; i < cells; i++) {
		p[i] = std::pow(2.0f, std::clamp(ev[i], -1.5f, 1.6f) / 2.2f);
		g.lo = std::min(g.lo, p[i]);
		g.hi = std::max(g.hi, p[i]);
	}
	// The lines: the gain against the brightness, over each 3 x 3 of blocks,
	// then each line the mean of the 3 x 3's (the guided filter's two means).
	auto box = [&](const std::vector<float> &in, std::vector<float> &out) {
		out.assign(cells, 0.0f);
		for (int by = 0; by < int(gh); by++)
			for (int bx = 0; bx < int(gw); bx++) {
				float sum = 0;
				int n = 0;
				for (int dy = -1; dy <= 1; dy++)
					for (int dx = -1; dx <= 1; dx++) {
						int yy = by + dy, xx = bx + dx;
						if (yy < 0 || xx < 0 || yy >= int(gh) || xx >= int(gw))
							continue;
						sum += in[size_t(yy) * gw + size_t(xx)];
						n++;
					}
				out[size_t(by) * gw + size_t(bx)] = sum / float(n);
			}
	};
	std::vector<float> ip(cells), ii(cells), mi, mp, mip, mii;
	for (size_t i = 0; i < cells; i++) {
		ip[i] = mean[i] * p[i];
		ii[i] = mean[i] * mean[i];
	}
	box(mean, mi);
	box(p, mp);
	box(ip, mip);
	box(ii, mii);
	const float eps = 0.0025f;
	std::vector<float> la(cells), lb(cells);
	for (size_t i = 0; i < cells; i++) {
		float var = std::max(0.0f, mii[i] - mi[i] * mi[i]);
		la[i] = (mip[i] - mi[i] * mp[i]) / (var + eps);
		lb[i] = mp[i] - la[i] * mi[i];
	}
	box(la, g.a);
	box(lb, g.b);
	return g;
}

// A photo finished in place (packed YUV 4:2:0), on `threads` cores.
inline void finish(uint8_t *yuv, unsigned w, unsigned h, const Finish &how, unsigned threads)
{
	uint8_t *y = yuv, *u = yuv + size_t(w) * h, *v = u + size_t(w / 2) * (h / 2);
	const unsigned cw2 = w / 2, ch2 = h / 2;
	// The noise the sharpening and the local tone go by: a merge's, else the
	// photo's own.
	float grain = how.grain > 0.3f ? how.grain : how.noise > 0.3f ? how.noise : 0.0f;
	if (grain <= 0.0f && (how.crisp > 0.01f || how.local > 0.01f))
		grain = estimate_noise(y, w, h);
	// The noise filter: each pixel the mean of its neighbours within the
	// noise (luma 3 x 3, colour 5 x 5), so edges stay where they are.
	if (how.noise > 0.3f) {
		const int t = std::max(2, int(std::lround(2.5f * how.noise)));
		std::vector<uint8_t> src(y, y + size_t(w) * h);
		in_bands(ch2, threads, [&](unsigned c0, unsigned c1) {
			for (unsigned py = 2 * c0; py < 2 * c1; py++) {
				if (py == 0 || py + 1 >= h)
					continue;
				for (unsigned px = 1; px + 1 < w; px++) {
					int c = src[size_t(py) * w + px], sum = 0, n = 0;
					for (int dy = -1; dy <= 1; dy++) {
						const uint8_t *row = &src[size_t(int(py) + dy) * w + px];
						for (int dx = -1; dx <= 1; dx++) {
							int q = row[dx];
							if (std::abs(q - c) <= t) {
								sum += q;
								n++;
							}
						}
					}
					y[size_t(py) * w + px] = uint8_t((sum + n / 2) / n);
				}
			}
		});
		const int tc = std::max(3, int(std::lround(3.0f * how.noise)));
		for (uint8_t *plane : { u, v }) {
			std::vector<uint8_t> c(plane, plane + size_t(cw2) * ch2);
			in_bands(ch2, threads, [&](unsigned c0, unsigned c1) {
				for (unsigned py = std::max(2u, c0); py < std::min(c1, ch2 - 2); py++)
					for (unsigned px = 2; px + 2 < cw2; px++) {
						int mid = c[size_t(py) * cw2 + px], sum = 0, n = 0;
						for (int dy = -2; dy <= 2; dy++) {
							const uint8_t *row = &c[size_t(int(py) + dy) * cw2 + px];
							for (int dx = -2; dx <= 2; dx++) {
								int q = row[dx];
								if (std::abs(q - mid) <= tc) {
									sum += q;
									n++;
								}
							}
						}
						plane[size_t(py) * cw2 + px] = uint8_t((sum + n / 2) / n);
					}
			});
		}
	}
	// The sharpening, on what the filter left (the filter's own noise first:
	// what it took away is not there to be cored).
	if (how.crisp > 0.01f)
		sharpen(yuv, w, h, how.crisp, how.noise > 0.3f ? grain * 0.7f : grain, threads);
	// Near-grey toward neutral: a share of the cast, six levels at most.
	int su = 0, sv = 0;
	if (how.neutral > 0.01f) {
		float cu, cv;
		cast_of(yuv, w, h, cu, cv);
		su = int(std::lround(std::clamp(-how.neutral * cu, -6.0f, 6.0f)));
		sv = int(std::lround(std::clamp(-how.neutral * cv, -6.0f, 6.0f)));
	}
	// The tone, and the colour with it: as much more (or less) as its
	// brightness was made, a little less than in full, so the noise left in
	// the colour of the shadows is not made louder than the light.
	uint8_t table[256];
	tone_table(how.lift, how.toe, table);
	if (how.local <= 0.01f && !how.face) {
		float ratio[256];
		for (int i = 0; i < 256; i++)
			ratio[i] = std::clamp(std::pow((float(table[i]) + 8.0f) / (float(i) + 8.0f), 0.8f), 0.5f, 2.5f);
		in_bands(ch2, threads, [&](unsigned c0, unsigned c1) {
			for (unsigned cy = c0; cy < c1; cy++)
				for (unsigned cx = 0; cx < cw2; cx++) {
					size_t a = size_t(2 * cy) * w + 2 * cx;
					int mean = (y[a] + y[a + 1] + y[a + w] + y[a + w + 1] + 2) / 4;
					for (size_t i : { a, a + 1, a + w, a + w + 1 })
						y[i] = table[y[i]];
					float r = ratio[mean];
					size_t i = size_t(cy) * cw2 + cx;
					u[i] = uint8_t(std::clamp(std::lround(128.0f + (float(int(u[i]) - 128 + su)) * r), 0L, 255L));
					v[i] = uint8_t(std::clamp(std::lround(128.0f + (float(int(v[i]) - 128 + sv)) * r), 0L, 255L));
				}
		});
		return;
	}
	// With the local tone: each pixel's gain first (its block's line at its
	// brightness), then the table; the colour by its four pixels' change.
	const GainMap g = gain_map(y, w, h, how, grain, table, threads);
	// The gain's share left to a pixel toward white (so what is lifted
	// never clips), by its brightness.
	float keep[256];
	for (int i = 0; i < 256; i++)
		keep[i] = 1.0f - smooth_step(170.0f, 255.0f, float(i));
	// The colour's ratio for a change of brightness r (in 128ths, to 4).
	float ratio[513];
	for (int i = 0; i <= 512; i++)
		ratio[i] = std::clamp(std::pow(float(i) / 128.0f, 0.8f), 0.5f, 2.5f);
	// Where each column falls among the blocks.
	std::vector<unsigned> col(w);
	std::vector<float> frac(w);
	for (unsigned x = 0; x < w; x++) {
		float gx = std::clamp((float(x) + 0.5f) / float(g.block) - 0.5f, 0.0f, float(g.gw - 1));
		col[x] = std::min(unsigned(gx), g.gw > 1 ? g.gw - 2 : 0u);
		frac[x] = g.gw > 1 ? gx - float(col[x]) : 0.0f;
	}
	in_bands(ch2, threads, [&](unsigned c0, unsigned c1) {
		std::vector<float> ra(g.gw), rb(g.gw);
		std::vector<int> before(size_t(w) * 2), after(size_t(w) * 2);
		for (unsigned cy = c0; cy < c1; cy++) {
			for (unsigned k = 0; k < 2; k++) {
				const unsigned py = 2 * cy + k;
				// This row's lines, between the block rows either side.
				float gy = std::clamp((float(py) + 0.5f) / float(g.block) - 0.5f, 0.0f, float(g.gh - 1));
				unsigned r0 = std::min(unsigned(gy), g.gh > 1 ? g.gh - 2 : 0u);
				float fy = g.gh > 1 ? gy - float(r0) : 0.0f;
				unsigned r1 = std::min(r0 + 1, g.gh - 1);
				for (unsigned bx = 0; bx < g.gw; bx++) {
					ra[bx] = g.a[size_t(r0) * g.gw + bx] + fy * (g.a[size_t(r1) * g.gw + bx] - g.a[size_t(r0) * g.gw + bx]);
					rb[bx] = g.b[size_t(r0) * g.gw + bx] + fy * (g.b[size_t(r1) * g.gw + bx] - g.b[size_t(r0) * g.gw + bx]);
				}
				uint8_t *row = y + size_t(py) * w;
				for (unsigned x = 0; x < w; x++) {
					const unsigned c = col[x], c1x = std::min(c + 1, g.gw - 1);
					const float f = frac[x];
					float la = ra[c] + f * (ra[c1x] - ra[c]), lb = rb[c] + f * (rb[c1x] - rb[c]);
					const int s = row[x];
					float gain = std::clamp(la * (float(s) / 255.0f) + lb, g.lo, g.hi);
					if (gain > 1.0f)
						gain = 1.0f + (gain - 1.0f) * keep[s];
					int lit = std::min(255, int(float(s) * gain + 0.5f));
					before[size_t(k) * w + x] = s;
					row[x] = table[lit];
					after[size_t(k) * w + x] = row[x];
				}
			}
			for (unsigned cx = 0; cx < cw2; cx++) {
				int in = before[2 * cx] + before[2 * cx + 1] + before[w + 2 * cx] + before[w + 2 * cx + 1];
				int out = after[2 * cx] + after[2 * cx + 1] + after[w + 2 * cx] + after[w + 2 * cx + 1];
				int at = std::min(512, (out + 32) * 128 / (in + 32));
				float r = ratio[at];
				size_t i = size_t(cy) * cw2 + cx;
				u[i] = uint8_t(std::clamp(std::lround(128.0f + (float(int(u[i]) - 128 + su)) * r), 0L, 255L));
				v[i] = uint8_t(std::clamp(std::lround(128.0f + (float(int(v[i]) - 128 + sv)) * r), 0L, 255L));
			}
		}
	});
}

// A small copy of a frame (box means), for the viewfinder while a stack is
// taken: `ow` x `oh` from `w` x `h`, packed YUV 4:2:0, its brightness put
// through `table` (the finish's tone, so it looks as the photo will).
inline void small_copy(const uint8_t *yuv, unsigned w, unsigned h, unsigned ow, unsigned oh, const uint8_t table[256], std::vector<uint8_t> &out)
{
	out.resize(size_t(ow) * oh * 3 / 2);
	const uint8_t *y = yuv, *u = yuv + size_t(w) * h, *v = u + size_t(w / 2) * (h / 2);
	auto box = [](const uint8_t *plane, unsigned pw, unsigned x0, unsigned x1, unsigned y0, unsigned y1) {
		uint32_t sum = 0, n = 0;
		unsigned sx = std::max(1u, (x1 - x0) / 3), sy = std::max(1u, (y1 - y0) / 3);
		for (unsigned yy = y0; yy < y1; yy += sy)
			for (unsigned xx = x0; xx < x1; xx += sx) {
				sum += plane[size_t(yy) * pw + xx];
				n++;
			}
		return uint8_t(n ? sum / n : 0);
	};
	for (unsigned oy = 0; oy < oh; oy++)
		for (unsigned ox = 0; ox < ow; ox++) {
			unsigned x0 = ox * w / ow, x1 = std::max(x0 + 1, (ox + 1) * w / ow), y0 = oy * h / oh, y1 = std::max(y0 + 1, (oy + 1) * h / oh);
			out[size_t(oy) * ow + ox] = table[box(y, w, x0, x1, y0, y1)];
		}
	uint8_t *ou = out.data() + size_t(ow) * oh, *ov = ou + size_t(ow / 2) * (oh / 2);
	for (unsigned oy = 0; oy < oh / 2; oy++)
		for (unsigned ox = 0; ox < ow / 2; ox++) {
			unsigned x0 = ox * w / ow, x1 = std::max(x0 + 1, (ox + 1) * w / ow), y0 = oy * h / oh, y1 = std::max(y0 + 1, (oy + 1) * h / oh);
			ou[size_t(oy) * (ow / 2) + ox] = box(u, w / 2, x0, x1, y0, y1);
			ov[size_t(oy) * (ow / 2) + ox] = box(v, w / 2, x0, x1, y0, y1);
		}
}

} // namespace stack
