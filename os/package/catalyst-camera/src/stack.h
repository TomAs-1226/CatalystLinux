// stack.h: Night and HDR (1.9), frames merged as they come, kept apart from
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

// Coarse to fine: a sixteenth (all of the reach), then a quarter (about
// what that found). Full size is refined against the merge (Stack::add).
inline Shift align(const Pyramid &first, const Pyramid &next)
{
	Shift s = search(first.sixteenth, next.sixteenth, 0, 0, REACH, 2, 1);
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

	// The second frame shows the noise: its differences to the first,
	// aligned, by brightness, the median of each (a pair's difference is
	// the root of two noises: so much is taken off).
	void measure(const uint8_t *yuv, Shift s)
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
				int m = mean(ysum[size_t(py) * w + px], yw[size_t(py) * w + px]);
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
				int m = mean(usum[size_t(py) * cw2 + px], cw[size_t(py) * cw2 + px]);
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
		measured = true;
	}

	// The shift at full size: within two pixels of the quarter's, on 64
	// patches of 32 x 32 against the merge so far.
	void refine(const uint8_t *y, Shift &s)
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
								size_t i = size_t(py) * w + px;
								sum += unsigned(std::abs(int(y[size_t(sy) * w + sx]) - int(mean(ysum[i], yw[i]))));
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

	bool any() const { return lift > 0.01f || toe > 0.01f || neutral > 0.01f || noise > 0.3f; }
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

// A photo finished in place (packed YUV 4:2:0), on `threads` cores.
inline void finish(uint8_t *yuv, unsigned w, unsigned h, const Finish &how, unsigned threads)
{
	uint8_t *y = yuv, *u = yuv + size_t(w) * h, *v = u + size_t(w / 2) * (h / 2);
	const unsigned cw2 = w / 2, ch2 = h / 2;
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
