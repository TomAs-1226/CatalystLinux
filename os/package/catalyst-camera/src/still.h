// still.h: what catalyst-camera decides while it takes a photo, kept apart
// from the camera so it can be checked without one (test/still-test.cpp,
// under qemu): how long to wait for the photo's frame before trying another
// way, which frame is the photo, the aspect's crop, and the timing marks.
//
// 0.9.3's photos could keep "hold still" up for ten seconds and more: a
// try whose frames never came (its request refused, or every frame marked
// bad) waited six seconds before the next way was tried, and a cancelled
// request was not even noticed. Now a refusal ends a try at once, a few bad
// frames end it, and the wait is a frame's worth of time, not six seconds.

#pragma once

#include <algorithm>
#include <chrono>
#include <cstdint>
#include <cstdlib>
#include <string>

namespace still {

// The IMX219's longest exposure at its full 3280 x 2464, by its kernel
// driver (drivers/media/i2c/imx219.c): a frame of at most 0xfffe lines,
// exposing up to four lines fewer, each line at most 0x7ff0 pixels long at
// 182.4 million pixels a second (two CSI-2 lanes): 65530 x 32752 / 182.4 MHz.
constexpr int64_t LONGEST_US = 11766000;

// A frame's length for an exposure (µs): the full sensor's fastest (15 a
// second), or the exposure and a little.
inline int64_t frame_us(int64_t exposure_us)
{
	return std::max<int64_t>(66667, exposure_us + 1000);
}

// How long to wait for a try's frames before giving it up (ms): the
// sensor's start in its new mode, then two frame lengths for each frame it
// may take (the pipeline keeps one raw frame, so frames come at every other
// one the sensor sends), the whole times `patience` (the last try, twice).
// The start was given a second in 0.9.4, a guess never measured: on the
// handheld every try ran out ("the camera gave no picture"). Four seconds;
// a try the pipeline refuses still ends at once, so a working one costs
// nothing for it.
inline long deadline_ms(int64_t exposure_us, int frames, int patience = 1)
{
	return long((4000 + 2 * frame_us(exposure_us) * std::max(1, frames) / 1000) * std::max(1, patience));
}

// ---------------------------------------------------------------- stacks ---
//
// Night and HDR (1.9): a run of frames at one exposure merged as they come
// (stack.h). Each frame is waited for on its own, so a stack whose frames
// stop coming ends with what it has, never waiting out the whole run.

// The most frames a stack takes (stack.h's sums hold 63).
const int STACK_MOST = 32;

// What merging a frame takes at the most besides its exposure (ms): the
// ISP, our copy, the alignment and the sums at the full size on the CM0's
// three cores (my own allowance: about 0.15 s by the development PC's
// reckoning of the A53, two and a half times over).
const long MERGE_MS = 400;

// Each frame's wait in a stack (ms): two of its frame lengths (the
// pipeline may let one go by), and the merge of the one before.
inline long stack_frame_ms(int64_t exposure_us)
{
	return long(2 * frame_us(exposure_us) / 1000) + MERGE_MS;
}

// The whole stack's bound (ms): the sensor's start, then each frame's wait.
inline long stack_ms(int64_t exposure_us, int frames)
{
	return 4000 + long(std::clamp(frames, 1, STACK_MOST)) * stack_frame_ms(exposure_us);
}

// How long a stack should take (ms), for the shell's "hold still": the
// first frame (the sensor's start and a frame), then each frame's length and
// a little for its merge.
inline long stack_expect_ms(int64_t exposure_us, int frames)
{
	return long((2 * frame_us(exposure_us) + int64_t(std::max(0, frames - 1)) * (frame_us(exposure_us) + 60000)) / 1000);
}

// Enough of a stack merged to keep it when the hand moves part way: a third
// of the frames asked for, and two at the least.
inline bool enough(int merged, int planned)
{
	return merged >= std::max(2, (planned + 2) / 3);
}

// What a stack keeps in memory at w x h (MB): the frame copied out (1.5
// bytes a pixel), the luma's sums and weights (3) and the colour's (1.25),
// rounded up.
inline long stack_mb(unsigned w, unsigned h)
{
	return long(uint64_t(w) * h * 23 / 4 / (1024 * 1024)) + 1;
}

// The size a stack is merged at, from what memory the system has free
// (MB, -1 unknown): the full 3280 x 2464 where its sums fit beside
// everything else (96 MB kept for the shell and the system, as detail
// mode's burst keeps) and a photo waiting to be written (12 MB); the
// binned 1640 x 1232 where only that fits; else no stack at all.
enum class StackSize { Full, Binned, None };

inline StackSize stack_size(long available_mb)
{
	if (available_mb < 0)
		return StackSize::Full;
	long room = available_mb - 96 - 12;
	if (room >= stack_mb(3280, 2464))
		return StackSize::Full;
	if (room >= stack_mb(1640, 1232))
		return StackSize::Binned;
	return StackSize::None;
}

// ---------------------------------------------------------- every photo ---
//
// 1.5.2: every photo from a few frames back to back, held in memory until
// it is written and merged then (stack.h's merge_frames).

// The most frames a photo takes.
const int MERGE_MOST = 6;

// How many frames a photo at w x h takes, of `asked`, from what memory the
// system has free (MB, -1 unknown): each frame is held (12 MB at the full
// size) beside everything else (96 MB kept for the shell and the system,
// as a stack keeps) and a photo waiting to be written (12 MB); as many as
// fit, and one (no merge) where fewer than two do.
inline int photo_frames(long available_mb, unsigned w, unsigned h, int asked)
{
	asked = std::clamp(asked, 1, MERGE_MOST);
	if (asked < 2 || available_mb < 0)
		return asked;
	long frame = long(uint64_t(w) * h * 3 / 2 / (1024 * 1024)) + 1;
	long fit = (available_mb - 96 - 12) / frame;
	return fit >= 2 ? int(std::min<long>(asked, fit)) : 1;
}

// Whether a frame's exposure is the one asked for (a tenth either way, or
// a fifth of a millisecond: the sensor counts in lines).
inline bool matches(int64_t took_us, int64_t asked_us)
{
	if (asked_us <= 0)
		return false;
	return std::llabs(took_us - asked_us) <= std::max<int64_t>(asked_us / 10, 200);
}

// A frame's state as libcamera gives it.
enum Status { Success, Startup, Error, Cancelled };

enum Verdict { Wait, Take, Fail };

// Which frame is the photo, judged one at a time.
struct Judge {
	int good = 0, bad = 0;
	std::string why;

	// The frames a try lets go by before taking whatever comes: with the
	// exposure fixed, two (an IMX219 may say its first frame after a mode
	// switch had not yet taken the new one); in auto, eight to settle; for
	// a long exposure none (a second one would double the wait).
	static int most(bool fixed, bool long_exposure)
	{
		return long_exposure ? 0 : fixed ? 2 : 8;
	}

	Verdict see(Status status, bool fixed, bool long_exposure, bool exposure_matches, bool converged)
	{
		switch (status) {
		case Cancelled:
			// Refused by the pipeline (a buffer it could not take): no frame
			// will ever come this way.
			why = "request cancelled";
			return Fail;
		case Error:
			if (++bad >= 3) {
				why = "frames came back bad";
				return Fail;
			}
			return Wait;
		case Startup:
			// Still settling, the pipeline says: the photo only if its
			// exposure was fixed and is the one it has.
			if (fixed && exposure_matches)
				return Take;
			return ++good > most(fixed, long_exposure) + 2 ? Take : Wait;
		case Success:
			break;
		}
		good++;
		if (fixed)
			return exposure_matches || good > most(true, long_exposure) ? Take : Wait;
		return (converged && good > 1) || good > most(false, long_exposure) ? Take : Wait;
	}
};

// A crop of a frame, in its pixels the right way up.
struct Crop {
	unsigned x = 0, y = 0, w = 0, h = 0;
};

// The middle of a `w` x `h` frame at `a`:`b` (0:0 or 4:3, the sensor's own
// shape: all of it). Its sides are multiples of four and its corner on even
// pixels, as YUV 4:2:0 needs.
inline Crop crop_for(unsigned w, unsigned h, unsigned a, unsigned b)
{
	Crop c{0, 0, w, h};
	if (a == 0 || b == 0 || (a == 4 && b == 3))
		return c;
	auto four = [](double v) { return unsigned((v + 2.0) / 4.0) * 4u; };
	if (double(a) * h >= double(b) * w) {
		c.h = std::min(h, four(double(w) * b / a));
	} else {
		c.w = std::min(w, four(double(h) * a / b));
	}
	c.x = ((w - c.w) / 2) & ~1u;
	c.y = ((h - c.h) / 2) & ~1u;
	return c;
}

// "aspect=3:2" read: a and b, or 0:0 for the whole frame.
inline void read_aspect(const std::string &value, unsigned &a, unsigned &b)
{
	a = b = 0;
	size_t colon = value.find(':');
	if (colon == std::string::npos)
		return;
	a = unsigned(std::strtoul(value.c_str(), nullptr, 10));
	b = unsigned(std::strtoul(value.c_str() + colon + 1, nullptr, 10));
	if (a == 0 || b == 0 || a > 64 || b > 64)
		a = b = 0;
}

// The timing marks of one photo: milliseconds since its order came, each
// named, in the order they happened ("room 0 stop 28 config 81 …").
struct Marks {
	std::chrono::steady_clock::time_point t0 = std::chrono::steady_clock::now();
	std::string line;

	long since() const
	{
		return long(std::chrono::duration_cast<std::chrono::milliseconds>(std::chrono::steady_clock::now() - t0).count());
	}

	void mark(const char *name)
	{
		mark(name, since());
	}

	void mark(const char *name, long ms)
	{
		if (!line.empty())
			line += ' ';
		line += name;
		line += ' ';
		line += std::to_string(ms);
	}
};

} // namespace still
