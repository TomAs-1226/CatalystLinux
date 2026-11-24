// compose-test: checks compose.h on its own, without a camera. Built with
// the image's cross compiler and run under qemu-aarch64, as stack-test is:
//
//   aarch64-linux-g++ -std=c++17 -O2 -I../src compose-test.cpp -lpthread -o compose-test
//   qemu-aarch64 -L <sysroot> ./compose-test
//
// - Light trails: a light crossing a dark, noisy scene leaves a streak
//   along all of its path, in its own colour, while the dark stays as dark
//   as one frame of it (a lighten with no margin greys it by the noise's
//   highs); a frame darker everywhere changes nothing; the margin grows
//   with the gain.
// - The viewfinder's copy keeps a streak a pixel wide at its brightness,
//   which an average (stack.h's small_copy) all but loses.
// - Burst: the sharper of two frames scores higher, the score falling as
//   the blur grows, a robot blurred alone lowers it, noise alone scores
//   about nothing; of ten frames each blurred its own amount the least
//   blurred is the one kept; a tie keeps the earlier.
// - Memory: a burst at the full size only where two full frames fit.

#include "compose.h"

#include <cmath>
#include <cstdio>
#include <cstring>

static int failed = 0;

static void check(bool ok, const char *what)
{
	std::printf("%s %s\n", ok ? "ok  " : "FAIL", what);
	if (!ok)
		failed++;
}

struct Noise {
	uint64_t s;
	float normal()
	{
		auto uni = [&] {
			s = s * 6364136223846793005ull + 1442695040888963407ull;
			return (float((s >> 40) & 0xFFFFFF) + 0.5f) / 16777216.0f;
		};
		float a = uni(), b = uni();
		return std::sqrt(-2.0f * std::log(a)) * std::cos(6.2831853f * b);
	}
};

static const unsigned W = 640, H = 480;

// A night street: dark, with a few dim shapes, noise of `sigma` in it, and
// a light (a robot's LED, a car's lamp) of `r` x `r` at (lx, ly) if asked,
// red in its colour.
static std::vector<uint8_t> night(float sigma, Noise &n, int lx = -1, int ly = -1, int r = 4)
{
	std::vector<uint8_t> yuv(size_t(W) * H * 3 / 2);
	for (unsigned y = 0; y < H; y++)
		for (unsigned x = 0; x < W; x++) {
			int v = 18 + int(10.0 * std::sin(x * 0.02) * std::cos(y * 0.03)) + ((x / 80 + y / 60) % 3 == 0 ? 14 : 0);
			if (lx >= 0 && int(x) >= lx && int(x) < lx + r && int(y) >= ly && int(y) < ly + r)
				v = 245;
			yuv[size_t(y) * W + x] = uint8_t(std::clamp(std::lround(float(v) + sigma * n.normal()), 0L, 255L));
		}
	uint8_t *u = yuv.data() + size_t(W) * H, *v = u + size_t(W / 2) * (H / 2);
	for (unsigned y = 0; y < H / 2; y++)
		for (unsigned x = 0; x < W / 2; x++) {
			bool lit = lx >= 0 && int(2 * x) + 1 >= lx && int(2 * x) < lx + r && int(2 * y) + 1 >= ly && int(2 * y) < ly + r;
			u[size_t(y) * (W / 2) + x] = lit ? 90 : uint8_t(std::clamp(std::lround(128.0f + 0.5f * sigma * n.normal()), 0L, 255L));
			v[size_t(y) * (W / 2) + x] = lit ? 230 : uint8_t(std::clamp(std::lround(128.0f + 0.5f * sigma * n.normal()), 0L, 255L));
		}
	return yuv;
}

static double mean_y(const uint8_t *y, unsigned x0, unsigned y0, unsigned x1, unsigned y1)
{
	double sum = 0;
	size_t n = 0;
	for (unsigned py = y0; py < y1; py++)
		for (unsigned px = x0; px < x1; px++) {
			sum += y[size_t(py) * W + px];
			n++;
		}
	return n ? sum / double(n) : 0.0;
}

// A scene with detail to be sharp or blurred: edges, stripes, a few blocks.
static std::vector<uint8_t> detailed(unsigned w, unsigned h)
{
	std::vector<uint8_t> y(size_t(w) * h);
	for (unsigned py = 0; py < h; py++)
		for (unsigned px = 0; px < w; px++) {
			int v = 90 + int(50.0 * std::sin(px * 0.35) * std::cos(py * 0.21));
			if ((px / 23 + py / 17) % 2 == 0)
				v += 60;
			if (px % 41 < 2)
				v = 230;
			y[size_t(py) * w + px] = uint8_t(std::clamp(v, 0, 255));
		}
	return y;
}

// Blurred along x by a box of `k` pixels (a pan, a robot moving), within
// [x0, x1) x [y0, y1) only if given.
static std::vector<uint8_t> smeared(const std::vector<uint8_t> &y, unsigned w, unsigned h, int k, unsigned x0 = 0, unsigned y0 = 0, unsigned x1 = 0, unsigned y1 = 0)
{
	if (x1 == 0) {
		x1 = w;
		y1 = h;
	}
	std::vector<uint8_t> out = y;
	if (k <= 1)
		return out;
	for (unsigned py = y0; py < y1; py++)
		for (unsigned px = x0; px < x1; px++) {
			int sum = 0;
			for (int d = 0; d < k; d++)
				sum += y[size_t(py) * w + std::min<unsigned>(w - 1, px + unsigned(d))];
			out[size_t(py) * w + px] = uint8_t(sum / k);
		}
	return out;
}

static void noisy(std::vector<uint8_t> &y, float sigma, Noise &n)
{
	for (uint8_t &p : y)
		p = uint8_t(std::clamp(std::lround(float(p) + sigma * n.normal()), 0L, 255L));
}

int main()
{
	using namespace compose;
	Noise n{ 7 };

	// ---- light trails ----
	{
		const float gain = 4.0f;
		const float sigma = 1.5f + 0.8f * gain;
		const int margin = lighten_margin(gain);
		std::vector<uint8_t> trails = night(sigma, n, 100, 200);
		std::vector<uint8_t> naive = trails;
		// The light crosses the street, a few pixels a frame, for 60 frames.
		for (int i = 1; i < 60; i++) {
			std::vector<uint8_t> f = night(sigma, n, 100 + 7 * i, 200 + i);
			lighten(trails.data(), f.data(), W, H, margin, 3);
			lighten(naive.data(), f.data(), W, H, 0, 3);
		}
		// Along its path: lit all the way.
		bool streak = true;
		for (int i = 0; i < 60; i++)
			streak &= trails[size_t(201 + i) * W + 101 + 7 * i] > 215;
		check(streak, "the light's path is a streak, lit all the way along");
		const uint8_t *u = trails.data() + size_t(W) * H, *v = u + size_t(W / 2) * (H / 2);
		size_t at = size_t((200 + 30) / 2) * (W / 2) + (100 + 7 * 30) / 2;
		check(u[at] < 110 && v[at] > 200, "the streak keeps the light's colour");
		// Away from it, the dark: about one frame's, not the noise's highs.
		std::vector<uint8_t> one = night(sigma, n);
		double dark = mean_y(trails.data(), 20, 20, 300, 160), was = mean_y(one.data(), 20, 20, 300, 160), grey = mean_y(naive.data(), 20, 20, 300, 160);
		std::printf("     the dark: one frame %.1f, trails %.1f, a lighten with no margin %.1f\n", was, dark, grey);
		check(std::fabs(dark - was) < 0.5, "the dark stays as dark as one frame: the noise's highs do not pile up");
		check(grey - was > 2.0 * sigma, "(with no margin they would: the reason for it)");
		// A frame darker everywhere changes nothing.
		std::vector<uint8_t> before = trails, dim = night(0, n);
		for (size_t i = 0; i < size_t(W) * H; i++)
			dim[i] = uint8_t(dim[i] / 2);
		size_t changed = lighten(trails.data(), dim.data(), W, H, margin, 3);
		check(changed == 0 && trails == before, "a frame darker everywhere changes nothing");
		check(lighten_margin(1.0f) < lighten_margin(4.0f) && lighten_margin(4.0f) < lighten_margin(10.0f) && lighten_margin(0.0f) >= 4 && lighten_margin(100.0f) <= 30, "the margin grows with the gain, within its bounds");

		// The viewfinder's copy: a streak a pixel wide, made small.
		const unsigned bw = 1640, bh = 1232;
		std::vector<uint8_t> big(size_t(bw) * bh * 3 / 2, 128);
		std::memset(big.data(), 20, size_t(bw) * bh);
		for (unsigned x = 0; x < bw; x++)
			big[size_t(600) * bw + x] = 250;
		std::vector<uint8_t> small, averaged;
		small_brightest(big.data(), bw, bh, 320, 240, small);
		uint8_t table[256];
		stack::tone_table(0, 0, table);
		stack::small_copy(big.data(), bw, bh, 320, 240, table, averaged);
		int kept = 0, lost = 0;
		for (unsigned y = 110; y < 125; y++) {
			kept = std::max(kept, int(small[size_t(y) * 320 + 160]));
			lost = std::max(lost, int(averaged[size_t(y) * 320 + 160]));
		}
		std::printf("     a streak a pixel wide, made small: %d kept, %d averaged\n", kept, lost);
		check(small.size() == 320 * 240 * 3 / 2 && kept >= 245 && lost < 120, "the viewfinder's copy keeps a thin streak at its brightness");
		check(small[10 * 320 + 10] == 20, "and the dark as it is");
	}

	// ---- burst ----
	{
		const unsigned w = 820, h = 616;
		const float gain = 6.0f;
		const float sigma = 1.5f + 0.8f * gain;
		const int floor = noise_floor(gain);
		std::vector<uint8_t> sharp = detailed(w, h);
		double last = 1e30;
		bool falling = true;
		for (int k : { 1, 2, 3, 5, 8 }) {
			std::vector<uint8_t> f = smeared(sharp, w, h, k);
			noisy(f, sigma, n);
			double s = sharpness(f.data(), w, h, floor, 3);
			std::printf("     blurred over %d: %.1f\n", k, s);
			falling &= s < last;
			last = s;
		}
		check(falling, "the score falls as the blur grows");
		// A robot blurred, the rest held still: lower than the robot sharp.
		std::vector<uint8_t> still_robot = sharp, moving = smeared(sharp, w, h, 9, 300, 200, 460, 330);
		noisy(still_robot, sigma, n);
		noisy(moving, sigma, n);
		check(sharpness(moving.data(), w, h, floor, 3) < sharpness(still_robot.data(), w, h, floor, 3), "a robot moving alone lowers the score");
		// Noise on a flat grey: about nothing (its steps are under the floor).
		std::vector<uint8_t> flat(size_t(w) * h, 100);
		noisy(flat, sigma, n);
		double flat_s = sharpness(flat.data(), w, h, floor, 3), detail_s = sharpness(still_robot.data(), w, h, floor, 3);
		std::printf("     noise alone %.2f, the scene %.1f\n", flat_s, detail_s);
		check(flat_s < detail_s * 0.02, "noise alone scores about nothing");
		check(sharpness(flat.data(), 1, 1, floor, 3) == 0.0, "a frame too small to have steps scores nothing");
		// Ten frames, each its own blur: the least blurred is kept.
		const int blur[10] = { 6, 4, 7, 3, 5, 2, 6, 4, 8, 5 };
		Best best;
		for (int i = 0; i < 10; i++) {
			std::vector<uint8_t> f = smeared(sharp, w, h, blur[i]);
			noisy(f, sigma, n);
			best.offer(i + 1, sharpness(f.data(), w, h, floor, 3));
		}
		std::printf("     of ten, frame %d kept\n", best.at);
		check(best.at == 6, "of ten frames the least blurred is the one kept");
		Best tie;
		tie.offer(1, 50.0);
		check(!tie.offer(2, 50.0) && tie.at == 1 && tie.offer(3, 60.0) && tie.at == 3, "a tie keeps the earlier; sharper takes its place");
		check(noise_floor(1.0f) < noise_floor(8.0f) && noise_floor(0.0f) >= 2, "the noise floor grows with the gain");

		check(burst_size(-1) == still::StackSize::Full && burst_size(400) == still::StackSize::Full, "a burst at the full size where two frames fit");
		check(burst_size(120) == still::StackSize::Binned && burst_size(100) == still::StackSize::None, "binned where only that fits; else one frame");
	}

	std::printf("%s\n", failed ? "FAILED" : "all passed");
	return failed ? 1 : 0;
}
