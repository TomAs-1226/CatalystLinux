// still-test: checks still.h on its own, without a camera. Built with the
// image's cross compiler and run under qemu-aarch64, as photo-test is:
//
//   aarch64-linux-g++ -std=c++17 -O2 -I../src still-test.cpp -ljpeg -lpthread -o still-test
//   qemu-aarch64 -L <sysroot> ./still-test
//
// - Which frame is the photo: a cancelled request ends the try at once,
//   three bad frames end it, the fixed exposure's first matching frame is
//   taken, and auto waits for the exposure to settle (eight at most).
// - The waits are a few frames long (0.9.3 waited six seconds), and long
//   enough for the IMX219's longest exposure.
// - The aspects' crops of the full sensor and the binned fallback: even
//   corners, sides of four, centred; and a 3:2 crop made into a JPEG that
//   decodes at its size.
// - The timing marks read in order.

#include "photo.h"
#include "still.h"

#include <cstdio>
#include <cstring>

static int failed = 0;

static void check(bool ok, const char *what)
{
	std::printf("%s %s\n", ok ? "ok  " : "FAIL", what);
	if (!ok)
		failed++;
}

static bool decoded_size(const std::vector<uint8_t> &jpeg, unsigned &w, unsigned &h)
{
	jpeg_decompress_struct d;
	photo::JpegError e;
	d.err = jpeg_std_error(&e.mgr);
	e.mgr.error_exit = photo::jpeg_failed;
	if (setjmp(e.back)) {
		jpeg_destroy_decompress(&d);
		return false;
	}
	jpeg_create_decompress(&d);
	jpeg_mem_src(&d, jpeg.data(), static_cast<unsigned long>(jpeg.size()));
	jpeg_read_header(&d, TRUE);
	w = d.image_width;
	h = d.image_height;
	jpeg_destroy_decompress(&d);
	return true;
}

int main()
{
	using namespace still;
	{
		Judge j;
		check(j.see(Cancelled, true, false, false, false) == Fail && j.why == "request cancelled", "a cancelled request ends the try at once");
	}
	{
		Judge j;
		bool waited = j.see(Error, true, false, false, false) == Wait && j.see(Error, true, false, false, false) == Wait;
		check(waited && j.see(Error, true, false, false, false) == Fail, "three bad frames end the try");
	}
	{
		Judge j;
		check(j.see(Success, true, false, true, false) == Take, "fixed: the first frame with the exposure is the photo");
	}
	{
		Judge j;
		bool two = j.see(Success, true, false, false, false) == Wait && j.see(Success, true, false, false, false) == Wait;
		check(two && j.see(Success, true, false, false, false) == Take, "fixed: two frames without it at most, then the third");
	}
	{
		Judge j;
		check(j.see(Success, true, true, false, false) == Take, "a long exposure takes its first frame");
	}
	{
		Judge j;
		check(j.see(Startup, true, false, true, false) == Take, "a startup frame with the fixed exposure is the photo");
	}
	{
		Judge j;
		bool first = j.see(Success, false, false, false, true) == Wait;
		check(first && j.see(Success, false, false, false, true) == Take, "auto: one frame let go, then the settled one");
		Judge k;
		int n = 0;
		while (k.see(Success, false, false, false, false) == Wait && n < 20)
			n++;
		check(n == 8, "auto: never settling, the ninth frame");
	}
	check(deadline_ms(10000, 3) == 4400 && deadline_ms(10000, 1) == 4133, "a photo at 1/100 s waits 4.1 to 4.4 s at most for its first frame");
	check(deadline_ms(10000, 3, 2) == 8800, "the last try waits twice as long");
	check(deadline_ms(LONGEST_US, 1) > LONGEST_US / 1000 * 2, "the longest exposure is waited for");
	check(matches(16000, 16000) && matches(16100, 16000) && !matches(20000, 16000) && !matches(5000, 0), "an exposure matches within a tenth");

	// Night and HDR's stacks: each frame's wait grows with its exposure.
	check(stack_frame_ms(125000) == 2 * 126 + MERGE_MS && stack_frame_ms(1000000) == 2 * 1001 + MERGE_MS, "a stack's frame waits two of its lengths and a merge");
	check(stack_frame_ms(4000) == stack_frame_ms(60000), "short frames wait the sensor's fastest full-size frame");
	check(stack_frame_ms(1000000) > 2 * stack_frame_ms(125000) && stack_frame_ms(1000000) > 1000, "a one-second frame waits longer than it takes");
	check(stack_ms(125000, 8) == 4000 + 8 * stack_frame_ms(125000) && stack_ms(125000, 8) < 10000, "a hand-held night (8 x 1/8 s) is bounded under ten seconds");
	check(stack_ms(1000000, 4) > 4 * 1000000 / 1000 && stack_ms(1000000, 4) == 4000 + 4 * stack_frame_ms(1000000), "a braced night (4 x 1 s) is bounded past its four seconds");
	check(stack_ms(125000, 1000) == stack_ms(125000, STACK_MOST), "no more frames than a stack holds");
	{
		long hand = stack_expect_ms(125000, 8), braced = stack_expect_ms(1000000, 4);
		std::printf("     expected: 8 x 1/8 s %ld ms, 4 x 1 s %ld ms\n", hand, braced);
		check(hand >= 1000 && hand < stack_ms(125000, 8) && braced >= 4000 && braced < stack_ms(1000000, 4), "what a stack should take: its frames' time, and inside its bound");
	}
	check(!enough(1, 8) && !enough(2, 8) && enough(3, 8) && enough(2, 4) && !enough(1, 2) && enough(2, 2), "enough of a stack: a third of it, two frames at least");
	check(stack_mb(3280, 2464) == 45 && stack_mb(1640, 1232) == 12, "a stack's memory: 45 MB at full size, 12 binned");
	check(stack_size(400) == StackSize::Full && stack_size(96 + 12 + 45) == StackSize::Full, "room for the full size: merged at 3280 x 2464");
	check(stack_size(96 + 12 + 44) == StackSize::Binned && stack_size(96 + 12 + 12) == StackSize::Binned, "short of it: merged binned, at 1640 x 1232");
	check(stack_size(96 + 12 + 11) == StackSize::None && stack_size(-1) == StackSize::Full, "too short for either: no stack; not known: full");

	struct Case {
		unsigned W, H, a, b, x, y, w, h;
	} cases[] = {
		{ 3280, 2464, 4, 3, 0, 0, 3280, 2464 },
		{ 3280, 2464, 0, 0, 0, 0, 3280, 2464 },
		{ 3280, 2464, 3, 2, 0, 138, 3280, 2188 },
		{ 3280, 2464, 16, 9, 0, 310, 3280, 1844 },
		{ 3280, 2464, 1, 1, 408, 0, 2464, 2464 },
		{ 1640, 1232, 3, 2, 0, 70, 1640, 1092 },
		{ 1640, 1232, 16, 9, 0, 154, 1640, 924 },
		{ 1640, 1232, 1, 1, 204, 0, 1232, 1232 },
	};
	for (const Case &c : cases) {
		Crop r = crop_for(c.W, c.H, c.a, c.b);
		char what[96];
		std::snprintf(what, sizeof what, "%u x %u at %u:%u is %u x %u at (%u, %u)", c.W, c.H, c.a, c.b, r.w, r.h, r.x, r.y);
		bool shape = r.x % 2 == 0 && r.y % 2 == 0 && r.w % 4 == 0 && r.h % 4 == 0 && r.x + r.w <= c.W && r.y + r.h <= c.H;
		check(shape && r.x == c.x && r.y == c.y && r.w == c.w && r.h == c.h, what);
	}
	unsigned a = 9, b = 9;
	read_aspect("3:2", a, b);
	bool three_two = a == 3 && b == 2;
	read_aspect("nonsense", a, b);
	check(three_two && a == 0 && b == 0, "aspect words read, nonsense is the whole frame");

	// A 3:2 crop of a full frame, packed as catalyst-camera copies it, made
	// into a JPEG on three cores: it decodes at the crop's size.
	{
		const unsigned W = 3280, H = 2464;
		std::vector<uint8_t> full(size_t(W) * H * 3 / 2);
		for (size_t i = 0; i < full.size(); i++)
			full[i] = uint8_t((i * 7) >> 5);
		Crop r = crop_for(W, H, 3, 2);
		std::vector<uint8_t> out(size_t(r.w) * r.h * 3 / 2);
		for (unsigned y = 0; y < r.h; y++)
			std::memcpy(&out[size_t(y) * r.w], &full[size_t(r.y + y) * W + r.x], r.w);
		const uint8_t *u = full.data() + size_t(W) * H, *v = u + size_t(W / 2) * (H / 2);
		uint8_t *ou = out.data() + size_t(r.w) * r.h, *ov = ou + size_t(r.w / 2) * (r.h / 2);
		for (unsigned y = 0; y < r.h / 2; y++) {
			std::memcpy(ou + size_t(y) * (r.w / 2), u + size_t(r.y / 2 + y) * (W / 2) + r.x / 2, r.w / 2);
			std::memcpy(ov + size_t(y) * (r.w / 2), v + size_t(r.y / 2 + y) * (W / 2) + r.x / 2, r.w / 2);
		}
		std::vector<uint8_t> jpeg;
		std::string why;
		unsigned jw = 0, jh = 0;
		bool made = photo::encode(out.data(), r.w, r.h, 95, {}, 3, jpeg, why) && decoded_size(jpeg, jw, jh);
		check(made && jw == 3280 && jh == 2188, "a 3:2 photo is made in bands and decodes at 3280 x 2188");
	}

	Marks m;
	m.mark("room", 0);
	m.mark("stop", 28);
	m.mark("frame", 377);
	check(m.line == "room 0 stop 28 frame 377", "the timing marks read in order");

	std::printf("%s\n", failed ? "FAILED" : "all passed");
	return failed ? 1 : 0;
}
