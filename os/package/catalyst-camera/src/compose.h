// compose.h: light trails' lighten merge and burst's sharpness (1.10), kept
// apart from the camera so they can be checked without one
// (test/compose-test.cpp, under qemu).
//
// - Light trails. Frames of one scene, one after another, each laid over a
//   running picture wherever it is brighter: the brightest each pixel has
//   been wins, so a light that moves leaves a streak and what holds still
//   stays as it was (Olympus's Live Composite; an editor's "lighten"
//   blend). Only light counts, not the noise: the brightest of sixty noisy
//   frames is two of the noise's deviations over any one of them, and a
//   plain lighten greys the dark by that. So each two by two of the picture
//   (4:2:0's colour sample) is lit by a frame only where it is brighter by
//   well over the noise: its mean by `margin`, or one of its pixels by
//   twice that (three and six deviations, which the noise alone reaches
//   about once in a billion). Where it is, each of its four pixels keeps
//   the brighter of the two, and its colour is the new frame's where that
//   holds the brightest light the four have seen. Elsewhere nothing moves:
//   the dark stays exactly as dark as the first frame.
// - The viewfinder's copy of the trails keeps the brightest of each box: an
//   average would thin a streak a pixel wide to a tenth of itself at the
//   preview's size, and the trails are what is being watched.
// - Burst's sharpness: the brightness's gradient energy (each pixel's step
//   to the right and down, squared), less the noise floor's share, over the
//   whole frame. Blur (a hand, a robot crossing the field) takes the fine
//   steps first; what held still adds the same to every frame, so only what
//   changed between the frames decides between them. Only the sharpest so
//   far is kept ([`Best`]).
//
// Memory: trails keeps its running picture and the frame copied out, 1.5
// bytes a pixel each: 6 MB at the binned 1640 x 1232 it runs at (it needs
// every frame the sensor gives, which only the binned mode's buffers can
// keep up with). Burst keeps the sharpest frame so far and the one just
// copied out: 24 MB at the full 3280 x 2464 (burst_size: binned, 6 MB, when
// the system has not that to spare). Neither keeps anything else.

#pragma once

#include "stack.h"
#include "still.h"

#include <algorithm>
#include <atomic>
#include <cmath>
#include <cstdint>
#include <vector>

namespace compose {

// --------------------------------------------------------------- trails ---

// The longest light trails runs for (s): two minutes, my own bound (the
// picture is one buffer however long it runs; a longer one would only fill
// with the noise's highs).
const int TRAILS_LONGEST_S = 120;

// The noise's standard deviation at a gain (levels): a small sensor's, as
// stack.h measures it, about 1.5 at the least gain and 0.8 more for each
// step of gain.
inline float noise_at(float gain)
{
	return 1.5f + 0.8f * std::clamp(gain, 1.0f, 16.0f);
}

// How much brighter a two by two's mean must be to be lit (levels): three
// of the noise's deviations (its mean's six), between 4 and 30.
inline int lighten_margin(float gain)
{
	return std::clamp(int(std::lround(3.0f * noise_at(gain))), 4, 30);
}

// `frame` laid over `into` where it is brighter by more than the noise
// (both packed YUV 4:2:0, w x h; `margin` as lighten_margin, 0 for a plain
// lighten), on `threads` cores. Gives how many pixels it brightened.
inline size_t lighten(uint8_t *into, const uint8_t *frame, unsigned w, unsigned h, int margin, unsigned threads)
{
	const unsigned cw = w / 2, ch = h / 2;
	uint8_t *y = into, *u = into + size_t(w) * h, *v = u + size_t(cw) * ch;
	const uint8_t *fy = frame, *fu = frame + size_t(w) * h, *fv = fu + size_t(cw) * ch;
	std::atomic<size_t> changed{0};
	stack::in_bands(ch, threads, [&](unsigned c0, unsigned c1) {
		size_t n = 0;
		for (unsigned cy = c0; cy < c1; cy++)
			for (unsigned cx = 0; cx < cw; cx++) {
				const size_t a = size_t(2 * cy) * w + 2 * cx;
				const size_t at[4] = { a, a + 1, a + w, a + w + 1 };
				int was_sum = 0, now_sum = 0, was_max = 0, now_max = 0, rise = -255;
				for (size_t i : at) {
					was_sum += y[i];
					now_sum += fy[i];
					was_max = std::max(was_max, int(y[i]));
					now_max = std::max(now_max, int(fy[i]));
					rise = std::max(rise, int(fy[i]) - int(y[i]));
				}
				// Not lit by this frame (only its noise): as it was.
				if (rise <= 0 || (now_sum <= was_sum + 4 * margin && rise <= 2 * margin))
					continue;
				for (size_t i : at)
					if (fy[i] > y[i]) {
						y[i] = fy[i];
						n++;
					}
				// The colour of the brightest light the four have seen.
				if (now_max > was_max) {
					const size_t c = size_t(cy) * cw + cx;
					u[c] = fu[c];
					v[c] = fv[c];
				}
			}
		changed += n;
	});
	return changed.load();
}

// A small copy of the trails for the viewfinder: `ow` x `oh` from `w` x
// `h` (packed YUV 4:2:0), each pixel the brightest of its box (every other
// pixel each way looked at), each colour the one where the brightest of its
// box was.
inline void small_brightest(const uint8_t *yuv, unsigned w, unsigned h, unsigned ow, unsigned oh, std::vector<uint8_t> &out)
{
	out.resize(size_t(ow) * oh * 3 / 2);
	const uint8_t *y = yuv, *u = yuv + size_t(w) * h, *v = u + size_t(w / 2) * (h / 2);
	uint8_t *oy = out.data(), *ou = oy + size_t(ow) * oh, *ov = ou + size_t(ow / 2) * (oh / 2);
	// The brightest in [x0, x1) x [y0, y1), and where.
	auto brightest = [&](unsigned x0, unsigned x1, unsigned y0, unsigned y1, unsigned &bx, unsigned &by) {
		int best = -1;
		const unsigned step = (x1 - x0 > 2 || y1 - y0 > 2) ? 2 : 1;
		for (unsigned yy = y0; yy < y1; yy += step)
			for (unsigned xx = x0; xx < x1; xx += step) {
				int l = y[size_t(yy) * w + xx];
				if (l > best) {
					best = l;
					bx = xx;
					by = yy;
				}
			}
		return uint8_t(std::max(best, 0));
	};
	auto span = [](unsigned o, unsigned n, unsigned big) {
		unsigned a = unsigned(uint64_t(o) * big / n), b = unsigned(uint64_t(o + 1) * big / n);
		return std::pair<unsigned, unsigned>(a, std::max(a + 1, std::min(b, big)));
	};
	for (unsigned py = 0; py < oh; py++) {
		auto [y0, y1] = span(py, oh, h);
		for (unsigned px = 0; px < ow; px++) {
			auto [x0, x1] = span(px, ow, w);
			unsigned bx = x0, by = y0;
			oy[size_t(py) * ow + px] = brightest(x0, x1, y0, y1, bx, by);
		}
	}
	// Each colour sample stands for two by two of the copy's pixels: its
	// box is theirs together.
	for (unsigned py = 0; py < oh / 2; py++) {
		const unsigned y0 = span(2 * py, oh, h).first, y1 = span(2 * py + 1, oh, h).second;
		for (unsigned px = 0; px < ow / 2; px++) {
			const unsigned x0 = span(2 * px, ow, w).first, x1 = span(2 * px + 1, ow, w).second;
			unsigned bx = x0, by = y0;
			brightest(x0, x1, y0, y1, bx, by);
			const size_t c = size_t(std::min(by / 2, h / 2 - 1)) * (w / 2) + std::min(bx / 2, w / 2 - 1);
			ou[size_t(py) * (ow / 2) + px] = u[c];
			ov[size_t(py) * (ow / 2) + px] = v[c];
		}
	}
}

// ---------------------------------------------------------------- burst ---

// The most frames a burst takes, and how many by default: about ten, as a
// phone's burst does in a second (my own bound above it: each is copied
// and scored at the full size, a few tens of milliseconds).
const int BEST_MOST = 16;

// The steps the noise makes by itself at a gain (levels), which the
// sharpness leaves out: two and a half of its deviations (a step between
// two noisy pixels is that over the floor each way once in twenty or so,
// so noise alone scores a fiftieth of a scene's detail or less).
inline int noise_floor(float gain)
{
	return std::clamp(int(std::lround(2.5f * noise_at(gain))), 2, 30);
}

// A frame's sharpness: the mean over the picture (every other row and
// column of its brightness, `w` x `h`, packed) of each pixel's squared
// steps to the right and down, less what the noise floor's steps would
// give, never below none for a pixel.
inline double sharpness(const uint8_t *y, unsigned w, unsigned h, int floor, unsigned threads)
{
	if (w < 2 || h < 2)
		return 0.0;
	const int64_t less = 2 * int64_t(floor) * floor;
	std::atomic<int64_t> total{0};
	std::atomic<int64_t> seen{0};
	// Shared out by the rows looked at (every other one).
	stack::in_bands(h / 2, threads, [&](unsigned r0, unsigned r1) {
		int64_t sum = 0, n = 0;
		for (unsigned r = r0; r < r1; r++) {
			const unsigned py = 2 * r;
			if (py + 1 >= h)
				continue;
			const uint8_t *row = y + size_t(py) * w, *below = row + w;
			for (unsigned px = 0; px + 1 < w; px += 2) {
				int dx = int(row[px + 1]) - int(row[px]);
				int dy = int(below[px]) - int(row[px]);
				int64_t g = int64_t(dx) * dx + int64_t(dy) * dy - less;
				if (g > 0)
					sum += g;
				n++;
			}
		}
		total += sum;
		seen += n;
	});
	return seen.load() > 0 ? double(total.load()) / double(seen.load()) : 0.0;
}

// The sharpest frame so far: which (counted from 1) and its score. A frame
// offered is kept only when it is sharper by more than a hair (a tie keeps
// the earlier: it was nearer the shutter).
struct Best {
	int at = 0;
	double score = -1.0;

	bool offer(int frame, double s)
	{
		if (at != 0 && s <= score * 1.002)
			return false;
		at = frame;
		score = s;
		return true;
	}
};

// The size a burst is taken at, from what memory the system has free (MB,
// -1 unknown): two full frames (the sharpest and the next, 12 MB each)
// beside everything else (96 MB kept, as a stack keeps) and a photo waiting
// to be written (12 MB); else two binned ones; else no burst (one frame).
inline still::StackSize burst_size(long available_mb)
{
	if (available_mb < 0)
		return still::StackSize::Full;
	long room = available_mb - 96 - 12;
	auto frame = [](unsigned w, unsigned h) { return long(uint64_t(w) * h * 3 / 2 / (1024 * 1024)) + 1; };
	if (room >= 2 * frame(3280, 2464))
		return still::StackSize::Full;
	if (room >= 2 * frame(1640, 1232))
		return still::StackSize::Binned;
	return still::StackSize::None;
}

} // namespace compose
