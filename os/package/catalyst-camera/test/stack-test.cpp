// stack-test: checks stack.h on its own, without a camera. Built with the
// image's cross compiler and run under qemu-aarch64, as still-test is:
//
//   aarch64-linux-g++ -std=c++17 -O2 -I../src stack-test.cpp -lpthread -o stack-test
//   qemu-aarch64 -L <sysroot> ./stack-test
//
// - Alignment: frames of a scene shifted by a known amount, with a small
//   sensor's noise in them, are found at that shift; a different scene is
//   left out.
// - The noise is measured as it is, and stacking eight frames takes it
//   down by about the root of eight.
// - Something moving through the frames leaves no ghost: where it passed
//   only later, the merge is the background; where it stood in the first,
//   it stands.
// - The finish: no change asked, none made; a lift brightens the middle and
//   keeps white white; near-grey colour pulled toward neutral by a share,
//   six levels at most; the viewfinder's small copy.
// - Every photo (1.5.2): three noisy frames merged, the noise down by about
//   the root of three, what moved left as the first frame had it; the local
//   tone on a backlit scene (the dark subject lifted, the sky kept, no halo
//   at the hard edge between them, a face in shade brought up); the
//   sharpening (an edge crisper with no overshoot past 4 levels, flat noise
//   not made louder, texture on skin left nearly alone); a photo's own
//   noise measured from it.

#include "stack.h"

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

static const unsigned W = 640, H = 480;

// A scene with something to line up on: soft gradients, a few hard-edged
// blocks and fine stripes; its colour gentle.
static int scene_y(int x, int y)
{
	int v = 70 + (x * 60) / int(W) + (y * 40) / int(H);
	v += int(24.0 * std::sin(x * 0.07) * std::cos(y * 0.05));
	uint32_t s = 2463534242u;
	for (int i = 0; i < 24; i++) {
		s ^= s << 13;
		s ^= s >> 17;
		s ^= s << 5;
		int bx = int(s % W), by = int((s >> 10) % H), bw = 20 + int((s >> 20) % 60), bh = 20 + int((s >> 4) % 50);
		if (x >= bx && x < bx + bw && y >= by && y < by + bh)
			v += (i % 2 ? 50 : -45);
	}
	return std::clamp(v, 0, 255);
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

// The scene moved by (tx, ty) (its content at (x + tx, y + ty) is the
// still scene's at (x, y)), with noise of sigma in its brightness (half in
// its colour), and a square of `object` brightness at (ox, oy) if asked.
static std::vector<uint8_t> frame(int tx, int ty, float sigma, Noise &n, int ox = -1, int oy = -1, int object = 250)
{
	std::vector<uint8_t> yuv(size_t(W) * H * 3 / 2);
	for (unsigned y = 0; y < H; y++)
		for (unsigned x = 0; x < W; x++) {
			int sx = int(x) - tx, sy = int(y) - ty;
			int v = scene_y(std::clamp(sx, 0, int(W) - 1), std::clamp(sy, 0, int(H) - 1));
			if (ox >= 0 && int(x) >= ox && int(x) < ox + 40 && int(y) >= oy && int(y) < oy + 40)
				v = object;
			yuv[size_t(y) * W + x] = uint8_t(std::clamp(std::lround(float(v) + sigma * n.normal()), 0L, 255L));
		}
	uint8_t *u = yuv.data() + size_t(W) * H, *v = u + size_t(W / 2) * (H / 2);
	for (unsigned y = 0; y < H / 2; y++)
		for (unsigned x = 0; x < W / 2; x++) {
			u[size_t(y) * (W / 2) + x] = uint8_t(std::clamp(std::lround(120.0f + 0.5f * sigma * n.normal()), 0L, 255L));
			v[size_t(y) * (W / 2) + x] = uint8_t(std::clamp(std::lround(136.0f + 0.5f * sigma * n.normal()), 0L, 255L));
		}
	return yuv;
}

// How far a picture's brightness is from the still scene's (root mean
// square), away from the edges and outside a box left out.
static float rms(const uint8_t *y, int skip_x = -1, int skip_y = -1, int skip_w = 0, int skip_h = 0)
{
	double sum = 0;
	size_t n = 0;
	for (unsigned py = 40; py + 40 < H; py++)
		for (unsigned px = 40; px + 40 < W; px++) {
			if (int(px) >= skip_x && int(px) < skip_x + skip_w && int(py) >= skip_y && int(py) < skip_y + skip_h)
				continue;
			double d = double(y[size_t(py) * W + px]) - scene_y(int(px), int(py));
			sum += d * d;
			n++;
		}
	return float(std::sqrt(sum / double(n)));
}

int main()
{
	using namespace stack;
	Noise n{ 42 };
	const float sigma = 8.0f;

	// Alignment: each frame found where it was moved to.
	{
		Stack st(W, H);
		std::vector<uint8_t> first = frame(0, 0, sigma, n);
		st.first(first.data());
		const int shifts[][2] = { { 13, -7 }, { -21, 9 }, { 3, 2 }, { 0, 0 }, { -40, -30 }, { 57, 11 } };
		bool all = true;
		for (auto &s : shifts) {
			std::vector<uint8_t> f = frame(s[0], s[1], sigma, n);
			Shift found;
			bool merged = st.add(f.data(), 3, &found);
			if (!merged || found.dx != s[0] || found.dy != s[1]) {
				std::printf("     shift (%d, %d) found (%d, %d) err %.1f merged %d\n", s[0], s[1], found.dx, found.dy, double(found.err), merged);
				all = false;
			}
		}
		check(all, "frames shifted up to 57 pixels, noisy, are found where they were moved to");
		bool measured = st.measured && std::fabs(st.noise[6] - sigma) < 0.25f * sigma;
		std::printf("     noise measured %.2f (sigma %.1f), colour %.2f\n", double(st.noise[6]), double(sigma), double(st.colour_noise));
		check(measured, "the noise is measured from the second frame, within a quarter");
		// Another scene altogether (the hand swung away): left out.
		std::vector<uint8_t> other(size_t(W) * H * 3 / 2);
		for (size_t i = 0; i < other.size(); i++)
			other[i] = uint8_t((i * 2654435761u) >> 24);
		int before = st.frames;
		check(!st.add(other.data(), 3) && st.frames == before, "a frame that will not line up is left out whole");
	}

	// Stacking takes the noise down by about the root of the frames.
	{
		Stack st(W, H);
		std::vector<uint8_t> f0 = frame(0, 0, sigma, n);
		st.first(f0.data());
		float one = rms(f0.data());
		for (int i = 1; i < 8; i++) {
			std::vector<uint8_t> f = frame((i * 5) % 11 - 5, (i * 3) % 7 - 3, sigma, n);
			st.add(f.data(), 3);
		}
		std::vector<uint8_t> out(size_t(W) * H * 3 / 2);
		st.result(out.data());
		float eight = rms(out.data());
		std::printf("     one frame %.2f, eight merged %.2f (root of eight: %.2f)\n", double(one), double(eight), double(one / std::sqrt(8.0f)));
		check(st.frames == 8 && eight < one * 0.45f, "eight frames merged: the noise down by more than half (about the root of eight)");
		check(st.residual() < sigma / 2.5f, "the noise left is known to the finish");
	}

	// Something moving: a bright square that crosses the picture.
	{
		Stack st(W, H);
		std::vector<uint8_t> f0 = frame(0, 0, sigma, n, 100, 200);
		st.first(f0.data());
		for (int i = 1; i < 8; i++) {
			std::vector<uint8_t> f = frame(0, 0, sigma, n, 100 + 60 * i, 200);
			st.add(f.data(), 3);
		}
		std::vector<uint8_t> out(size_t(W) * H * 3 / 2);
		st.result(out.data());
		// Where it passed only later: the background, no ghost.
		float ghost = 0;
		for (int i = 1; i < 8; i++) {
			int ox = 100 + 60 * i;
			if (ox + 40 > int(W) - 40)
				break;
			double sum = 0;
			int k = 0;
			for (int y = 205; y < 235; y++)
				for (int x = ox + 5; x < ox + 35; x++) {
					if (x >= 100 && x < 140)
						continue;
					sum += std::fabs(double(out[size_t(y) * W + x]) - scene_y(x, y));
					k++;
				}
			if (k)
				ghost = std::max(ghost, float(sum / k));
		}
		// Where it stood in the first frame: it stands there.
		double stood = 0;
		for (int y = 205; y < 235; y++)
			for (int x = 105; x < 135; x++)
				stood += out[size_t(y) * W + x];
		stood /= 30 * 30;
		std::printf("     ghost %.1f levels from the background, the first frame's square %.0f\n", double(ghost), stood);
		check(ghost < 6.0f, "where it passed only in later frames, the background (no ghost)");
		check(stood > 235.0, "where it stood in the first frame, it stands");
		float rest = rms(out.data(), 90, 190, 500, 60);
		check(rest < sigma * 0.5f, "everywhere else, merged as usual");
	}

	// The finish.
	{
		uint8_t t[256];
		tone_table(0, 0, t);
		bool same = true;
		for (int i = 0; i < 256; i++)
			same &= std::abs(int(t[i]) - i) <= 1;
		check(same, "no lift, no toe: the tone is left as it was");
		tone_table(1.5f, 0.25f, t);
		std::printf("     a lift of 1.5 stops: 30 -> %d, 118 -> %d, 200 -> %d, 255 -> %d\n", t[30], t[118], t[200], t[255]);
		check(t[118] > 150 && t[255] == 255 && t[250] > t[200] && t[30] > 50, "a lift brightens the middle and the shadows, white stays white, the order kept");

		// An orange-tinged grey card, pulled toward neutral by half, six at most.
		std::vector<uint8_t> yuv(size_t(W) * H * 3 / 2, 128);
		std::memset(yuv.data(), 120, size_t(W) * H);
		uint8_t *u = yuv.data() + size_t(W) * H, *v = u + size_t(W / 2) * (H / 2);
		std::memset(u, 128 - 8, size_t(W / 2) * (H / 2));
		std::memset(v, 128 + 16, size_t(W / 2) * (H / 2));
		Finish f;
		f.neutral = 0.5f;
		finish(yuv.data(), W, H, f, 3);
		check(u[1000] == 128 - 4 && v[1000] == 128 + 10, "near-grey pulled toward neutral: half of the cast, six levels at most");

		std::vector<uint8_t> small;
		tone_table(0, 0, t);
		std::vector<uint8_t> f0 = frame(0, 0, 0, n);
		small_copy(f0.data(), W, H, 320, 240, t, small);
		check(small.size() == 320 * 240 * 3 / 2 && std::abs(int(small[120 * 320 + 160]) - scene_y(320, 240)) < 20, "the viewfinder's copy is the frame made small");
	}

	// Every photo (1.5.2): three frames, a hand's shift apart, merged into
	// the first: the noise down by about the root of three; what moved
	// stands as the first frame had it.
	{
		const float s = 6.0f;
		std::vector<uint8_t> f0 = frame(0, 0, s, n, 100, 200), f1 = frame(3, -2, s, n, 160, 200), f2 = frame(-5, 4, s, n, 220, 200);
		float one = rms(f0.data(), 90, 190, 200, 60);
		Merged m = merge_frames(f0.data(), { f1.data(), f2.data() }, W, H, 3);
		float three = rms(f0.data(), 90, 190, 200, 60);
		std::printf("     one frame %.2f, three merged %.2f (root of three: %.2f), noise %.2f left %.2f\n", double(one), double(three), double(one / std::sqrt(3.0f)), double(m.noise), double(m.residual));
		check(m.frames == 3 && three < one / 1.5f, "a photo's three frames merged: the noise down by about the root of three");
		check(m.residual < m.noise / 1.4f && m.residual > m.noise / 2.0f, "the noise left is known to the finish");
		double stood = 0;
		for (int y = 205; y < 235; y++)
			for (int x = 105; x < 135; x++)
				stood += f0[size_t(y) * W + x];
		stood /= 30 * 30;
		check(stood > 235.0, "what moved stands as the first frame had it (no ghost)");
		// A frame of another scene: left out, the photo is the first.
		std::vector<uint8_t> g0 = frame(0, 0, s, n), other(size_t(W) * H * 3 / 2);
		for (size_t i = 0; i < other.size(); i++)
			other[i] = uint8_t((i * 2654435761u) >> 24);
		std::vector<uint8_t> before = g0;
		Merged none = merge_frames(g0.data(), { other.data() }, W, H, 3);
		check(none.frames == 1 && g0 == before, "a frame that will not line up is left out; the photo is its first frame");
	}

	// The local tone on a backlit scene: a dark subject (40, textured) left
	// of a hard edge, a bright sky (215, textured) right of it.
	{
		auto backlit = [&](std::vector<uint8_t> &yuv) {
			yuv.assign(size_t(W) * H * 3 / 2, 128);
			for (unsigned y = 0; y < H; y++)
				for (unsigned x = 0; x < W; x++) {
					int base = x < W / 2 ? 40 : 215;
					int tex = int(6.0 * std::sin(x * 0.31) * std::sin(y * 0.23));
					yuv[size_t(y) * W + x] = uint8_t(base + tex);
				}
		};
		auto mean = [&](const std::vector<uint8_t> &yuv, unsigned x0, unsigned x1) {
			double sum = 0;
			for (unsigned y = 40; y < H - 40; y++)
				for (unsigned x = x0; x < x1; x++)
					sum += yuv[size_t(y) * W + x];
			return sum / double((H - 80) * (x1 - x0));
		};
		auto spread = [&](const std::vector<uint8_t> &yuv, unsigned x0, unsigned x1) {
			double m = mean(yuv, x0, x1), sum = 0;
			for (unsigned y = 40; y < H - 40; y++)
				for (unsigned x = x0; x < x1; x++)
					sum += (yuv[size_t(y) * W + x] - m) * (yuv[size_t(y) * W + x] - m);
			return std::sqrt(sum / double((H - 80) * (x1 - x0)));
		};
		std::vector<uint8_t> was, yuv;
		backlit(was);
		yuv = was;
		Finish f;
		f.local = 1.0f;
		finish(yuv.data(), W, H, f, 3);
		const unsigned e = W / 2;
		double dark = mean(yuv, 40, e - 80), near_dark = mean(yuv, e - 24, e - 2);
		double sky = mean(yuv, e + 80, W - 40), near_sky = mean(yuv, e + 2, e + 24);
		std::printf("     subject 40 -> %.1f (by the edge %.1f), sky 215 -> %.1f (by the edge %.1f), texture %.1f -> %.1f\n", dark, near_dark, sky, near_sky, spread(was, 40, e - 80), spread(yuv, 40, e - 80));
		check(dark > 46.0 && dark < 70.0, "the dark subject lifted (about half a stop), not to the middle");
		check(std::fabs(sky - mean(was, e + 80, W - 40)) < 2.0, "the bright sky kept as it was");
		check(std::fabs(near_dark - dark) < 4.0 && std::fabs(near_sky - sky) < 4.0, "no halo: by the hard edge, each side as it is further off (within 4 levels)");
		check(spread(yuv, 40, e - 80) > spread(was, 40, e - 80) * 1.1, "the subject's texture lifted with it, not flattened");
		// A lift asked (the frames kept under for the sky): the sky's held.
		yuv = was;
		f.lift = 1.0f;
		finish(yuv.data(), W, H, f, 3);
		Finish global;
		global.lift = 1.0f;
		std::vector<uint8_t> plain = was;
		finish(plain.data(), W, H, global, 3);
		std::printf("     with a lift of 1: sky %.1f (the lift alone %.1f), subject %.1f (alone %.1f)\n", mean(yuv, e + 80, W - 40), mean(plain, e + 80, W - 40), mean(yuv, 40, e - 80), mean(plain, 40, e - 80));
		check(mean(yuv, e + 80, W - 40) < mean(plain, e + 80, W - 40) - 1.0 && mean(yuv, 40, e - 80) >= mean(plain, 40, e - 80), "with a lift, the bright sky held back from it, the subject not");
		// A face in the shade: brought toward a face's level.
		yuv = was;
		Finish fa;
		fa.local = 1.0f;
		fa.face = true;
		fa.face_box[0] = 0.15f, fa.face_box[1] = 0.3f, fa.face_box[2] = 0.35f, fa.face_box[3] = 0.7f;
		finish(yuv.data(), W, H, fa, 3);
		double face = 0;
		for (unsigned y = unsigned(0.4f * H); y < unsigned(0.6f * H); y++)
			for (unsigned x = unsigned(0.2f * W); x < unsigned(0.3f * W); x++)
				face += yuv[size_t(y) * W + x];
		face /= double(unsigned(0.2f * H) * unsigned(0.1f * W));
		std::printf("     a face in the shade: %.1f (without it %.1f)\n", face, dark);
		check(face > dark + 8.0 && face < 120.0, "a face in the shade brought toward a face's level, not past it");
		check(std::fabs(mean(yuv, e + 80, W - 40) - mean(was, e + 80, W - 40)) < 2.0, "the sky behind it kept");
	}

	// Sharpening: a soft edge (top third), a flat noisy grey (middle third),
	// and a fine texture (bottom third), grey on its left half and in a skin
	// tone (Cb 103, Cr 157) on its right.
	{
		std::vector<uint8_t> yuv(size_t(W) * H * 3 / 2, 128);
		uint8_t *u = yuv.data() + size_t(W) * H, *v = u + size_t(W / 2) * (H / 2);
		for (unsigned y = 0; y < H / 3; y++)
			for (unsigned x = 0; x < W; x++) {
				double t = 0.5 * (1.0 + std::erf((double(x) - 320.0) / (1.2 * std::sqrt(2.0))));
				yuv[size_t(y) * W + x] = uint8_t(std::lround(60.0 + 120.0 * t));
			}
		for (unsigned y = H / 3; y < 2 * H / 3; y++)
			for (unsigned x = 0; x < W; x++)
				yuv[size_t(y) * W + x] = uint8_t(std::clamp(std::lround(128.0f + 3.0f * n.normal()), 0L, 255L));
		for (unsigned y = 2 * H / 3; y < H; y++)
			for (unsigned x = 0; x < W; x++)
				yuv[size_t(y) * W + x] = uint8_t(150 + int(20.0 * std::sin(x * 1.1) * std::sin(y * 0.9)));
		for (unsigned cy = H / 3; cy < H / 2; cy++)
			for (unsigned cx = W / 4; cx < W / 2; cx++) {
				u[size_t(cy) * (W / 2) + cx] = 103;
				v[size_t(cy) * (W / 2) + cx] = 157;
			}
		const std::vector<uint8_t> was = yuv;
		sharpen(yuv.data(), W, H, 1.0f, 3.0f, 3);
		auto steepest = [&](const std::vector<uint8_t> &p) {
			int most = 0;
			for (unsigned x = 300; x < 340; x++)
				most = std::max(most, int(p[size_t(80) * W + x + 1]) - int(p[size_t(80) * W + x]));
			return most;
		};
		int lo = 255, hi = 0;
		for (unsigned y = 10; y < H / 3 - 10; y++)
			for (unsigned x = 280; x < 360; x++) {
				lo = std::min(lo, int(yuv[size_t(y) * W + x]));
				hi = std::max(hi, int(yuv[size_t(y) * W + x]));
			}
		std::printf("     the edge's steepest step %d -> %d, its range %d to %d (was 60 to 180)\n", steepest(was), steepest(yuv), lo, hi);
		check(steepest(yuv) >= steepest(was) * 6 / 5, "the edge crisper: its steepest step up by a fifth or more");
		check(lo >= 60 - 4 && hi <= 180 + 4, "no halo: the edge never goes past its two sides by more than 4 levels");
		auto rough = [&](const std::vector<uint8_t> &p, unsigned x0, unsigned x1, unsigned y0, unsigned y1) {
			double sum = 0, sq = 0, k = 0;
			for (unsigned y = y0; y < y1; y++)
				for (unsigned x = x0; x < x1; x++) {
					double d = p[size_t(y) * W + x];
					sum += d;
					sq += d * d;
					k++;
				}
			return std::sqrt(sq / k - (sum / k) * (sum / k));
		};
		double flat0 = rough(was, 8, W - 8, H / 3 + 4, 2 * H / 3 - 4), flat1 = rough(yuv, 8, W - 8, H / 3 + 4, 2 * H / 3 - 4);
		std::printf("     flat noise %.2f -> %.2f\n", flat0, flat1);
		check(flat1 <= flat0 * 1.05, "the flat area's noise not made louder");
		double grey0 = rough(was, 8, W / 2 - 8, 2 * H / 3 + 4, H - 4), grey1 = rough(yuv, 8, W / 2 - 8, 2 * H / 3 + 4, H - 4);
		double skin0 = rough(was, W / 2 + 8, W - 8, 2 * H / 3 + 4, H - 4), skin1 = rough(yuv, W / 2 + 8, W - 8, 2 * H / 3 + 4, H - 4);
		std::printf("     texture in grey %.2f -> %.2f, the same in skin %.2f -> %.2f\n", grey0, grey1, skin0, skin1);
		check(grey1 > grey0 * 1.08, "texture in grey sharpened");
		check(skin1 - skin0 < (grey1 - grey0) / 3.0, "the same texture on skin left nearly alone (under a third as much)");
	}

	// Noise measured from a photo alone (no merge to measure it).
	{
		std::vector<uint8_t> f = frame(0, 0, 4.0f, n);
		float est = estimate_noise(f.data(), W, H);
		std::printf("     a photo's own noise: %.2f (sigma 4)\n", double(est));
		check(est > 2.5f && est < 5.5f, "a photo's own noise measured from it, near enough");
	}

	std::printf("%s\n", failed ? "FAILED" : "all passed");
	return failed ? 1 : 0;
}
