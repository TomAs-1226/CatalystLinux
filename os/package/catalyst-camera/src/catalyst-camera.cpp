// catalyst-camera: the camera's picture on standard output, for the shell's
// AprilTag reader (greyscale) and its Camera app (colour, and photos).
// Started when one of them opens and stopped when it closes (or stops
// reading).
//
//   catalyst-camera [--width 640] [--height 480] [--fps 30] [--slowest 15]
//                   [--orientation 180] [--colour]
//
// Greyscale (the default, for the AprilTag reader): each frame is the
// picture's brightness (the Y plane of YUV 4:2:0), one byte a pixel,
// width × height bytes with no padding and nothing between frames.
//
// Colour (--colour, for the Camera app): what comes out is a run of
// messages, each a 16-byte header ("CCAM", then three little-endian 32-bit
// numbers: its kind, the length of what follows, and a number of its own)
// and then that many bytes:
//
//   kind 1  a frame: YUV 4:2:0, the Y plane then U then V, each packed
//           (width × height, then a quarter of that twice); the number is
//           width << 16 | height
//   kind 2  a photo was saved: its path, then a tab and how long it took
//           (milliseconds: "shot 812 write 640"); the number is
//           width << 16 | height
//   kind 3  a photo could not be taken: why, in a line
//   kind 4  a photo's picture is in (the preview runs again while it is
//           written); the number is width << 16 | height, the line its path,
//           a tab and its timing marks (milliseconds since the order came:
//           "room 0 stop 28 config 81 start 199 frame 377 copy 405")
//   kind 6  the sensor is exposing the photo now: "<ms> <frames> <what>",
//           how long that should take at most, how many frames, and
//           "still", "detail" or "bracket" (the shell's "hold still")
//   kind 7  the sensor is done with the photo (or its burst or set): it
//           may move now; the line is the timing marks so far
//   kind 8  a stack's frames so far (Night and HDR, 1.9): "<taken>
//           <planned> <merged> <ms left>", after each frame
//   kind 5  what the camera said of the frame that follows, a line:
//           "exp=16000 ag=2.000 dg=1.000 r=1.9000 b=1.6000 ct=4800
//           lux=210.0 ae=1 fom=1234 zoom=0.00" (exposure µs, analogue and
//           digital gain, white balance gains, colour temperature, light,
//           exposure settled, focus figure of merit, the focus check's scale)
//
// and lines on standard input ask for things:
//
//   still <path>[\t<word>…]
//                     a photo at the sensor's full resolution, 3280 × 2464,
//                     as a JPEG at <path> (written beside it and renamed
//                     into place, so a half-written photo is never seen).
//                     The preview stops while the picture is taken and runs
//                     again while it is written. Words after it, tab apart,
//                     say what to take it with (the shell's pro.rs works
//                     them out from the preview's own frames):
//                       exp=<µs> gain=<g>  the exposure, fixed: the first
//                                          frame is the photo (nothing settles)
//                       gains=<r>,<b>      white balance gains, fixed
//                       ct=<K>             or a colour temperature
//                       sharp=<s>          libcamera's Sharpness
//                       nr=off|fast|hq     its NoiseReductionMode
//                       look=<file>        a look's table (filters.rs), laid
//                                          over the photo as it is written
//                       lookname=<name>    said in its Exif
//                       orig=<path>        the photo as taken, kept there
//                       raw=<path>         the sensor's own frame as a DNG
//                                          (catalyst-shell --write-dng)
//                       aspect=<a>:<b>     cropped to 3:2, 16:9 or 1:1 from
//                                          the middle (4:3: the whole frame)
//                       bias=<thirds>      its exposure compensation (Exif)
//                       program=<n>        Exif's ExposureProgram (1 manual,
//                                          2 normal, 4 shutter priority)
//                       also=<µs>,<g>,<thirds>,<path>
//                                          bracketing: one more frame of the
//                                          same session at that exposure,
//                                          saved there (a word for each)
//                       stack=<n>          Night and HDR (1.9): n frames at
//                                          the exposure given, merged as
//                                          they come (stack.h), at the full
//                                          size where memory has room for
//                                          its sums, else binned; the first
//                                          frame, made small, is shown in
//                                          the preview's place meanwhile
//                       stackfor=night|hdr what it is for (its Exif)
//                       lift=<stops> toe=<0..0.5> neutral=<0..1>
//                                          the finish (stack.h): brightness
//                                          given back, the shadows lifted,
//                                          near-grey pulled toward neutral
//                                          (also for a single photo)
//   detail auto|on <path>[\t<word>…]
//                     detail mode: the photo as above, then up to five more
//                     full-size frames with its exposure and white balance
//                     held, handed (in memory files) to the merge, which
//                     runs on by itself at idle priority and replaces the
//                     photo when done (catalyst-shell --merge-burst, see
//                     burst.rs). "auto" takes the burst only in real low
//                     light; the frames are as many as memory leaves room for.
//   face x0 y0 x1 y1  where a face is, 0 to 1 across this camera's picture
//   face none         (from the smart camera on the Grove port): the
//                     exposure leans toward it
//   stack end         a stack ends as soon as enough of it is merged (the
//                     hand moved: still.h's enough), and is kept
//   stack cancel      a stack ends at once with no photo (kind 3 "cancelled")
//                     (both read as they come, not after the photo)
//   night on|off      longer exposures allowed (down to 5 frames a second,
//                     a fifth of a second for a photo) when the light is low;
//                     into the next request, as the dials are (it restarted
//                     the camera until 1.9: the picture went dark and its
//                     exposure started again each time)
//   pro exp=<µs|auto> gain=<g|auto> ev=<stops> ct=<K|auto> max=<µs>
//       [gains=<r>,<b>] [meter=spot]
//                     the preview's exposure and white balance (pro mode's
//                     dials): fixed where given, the camera's own where auto;
//                     white balance gains (the grey card's) before ct; the
//                     exposure metered on the middle alone (the lock, 1.9)
//   focus on <x> <y>  the focus check: the preview from the full sensor,
//   focus off         cropped one pixel to one around (x, y) (0 to 1 of
//                     the picture); the binned mode cropped (half scale)
//                     where the full one will not run
//
// The sensor runs in its full-view 2 × 2 binned mode where it has one (the
// IMX219's 1640 × 1232), scaled by the ISP, so the field of view and the
// lens are known; libcamera picks otherwise.
//
// Memory: every camera buffer comes from the kernel's contiguous pool (CMA,
// 128 MB since 1.1.0, shared with the GPU, which keeps 16 MB of it for its
// binner while the shell draws with it). libcamera's Raspberry Pi pipeline
// keeps four raw frames of its own by default: at the full 3280 × 2464 that
// is 39 MB, and with the ISP's output frame and ours (12 MB each) 63 MB. The
// pipeline is told to keep two (vc4.yaml beside this program, through
// LIBCAMERA_RPI_CONFIG_FILE): a photo needs about 45 MB. One was too few: the
// sensor had nowhere to write while the ISP held it, and no photo ever came
// (1.1.1). The
// preview gives the sensor raw frames of ours in each request instead, so it
// still never waits for one. The photo's frame is copied out of the camera's
// memory at once and the camera handed back to the preview; the JPEG is
// written from the copy, on a thread of its own at a lower priority.
//
// Photos: the ISP's own 4:2:0 written straight as JPEG at quality 95 (no
// colour conversion, nothing resampled; 4:4:4 would need the ISP to give
// 24-bit frames, two of 24 MB, which the pool cannot hold beside the raw
// frame), and Exif: the camera, the lens, the exposure, the time, the look.
//
// Fast (1.4): a photo is taken with the exposure, gain and white balance
// the preview had settled on, fixed in the request that starts the full
// sensor, so its first frame is the photo (1.3 let up to eight frames
// settle again, at 15 a second, with one buffer: most of its 812 ms). The
// noise filter and sharpening are the shell's for the gain (off and firmer
// at little gain, where the filter only smears detail). The photo's frame
// comes from memory held while the camera is open (a buffer from the
// kernel's CMA heap, imported by the ISP), so taking one never waits on
// the pool for it; "captured" is said as soon as it is copied out. The
// JPEG is made on three cores in bands joined with restart markers
// (photo.h), behind the preview, and photos queue to be written: the next
// can be taken before the last is on the card. With RAW, the sensor's frame
// comes in the same request and is written as a DNG by the shell; the
// pipeline keeps no raw frame of its own then (vc4.yaml), so it takes no
// more of the pool than a JPEG photo.
//
// Bounded (1.5, 0.9.4): every wait in a photo is a few frames long, not six
// seconds; a request the pipeline refuses, or frames that come back bad,
// end a try at once; the held frame, once it fails, is not tried again
// this boot (/run/catalyst-camera.held-off); waiting for room to keep a
// photo is bounded too. Each photo's timing marks go to the shell (kind 4)
// and to standard error, as does any wait that ran out. still.h has the
// decisions, checked without a camera.
//
// Frames come at up to --fps a second, and no slower than --slowest: in dim
// light the exposure may take longer than a frame at --fps allows.
//
// Always the newest picture: each frame is copied out of the camera's buffer
// as it arrives and the buffer goes straight back to the camera, and the
// newest copy is what is written when the reader is ready for one. Frames the
// reader was too busy for are dropped, never queued, so what it sees is never
// late.
//
// Which way up: on the CardputerZero the IMX219 sits the other way round from
// what the kernel's imx219 overlay says (it declares the Raspberry Pi camera
// board's 180°, which libcamera undoes), so without a turn the picture is
// upside down. The turn is asked of the camera pipeline (the sensor reads out
// flipped, at no cost); where it cannot, it is done here as each frame is
// copied (and as a photo is written). --orientation 0 for a camera mounted as
// its overlay says; "mirror" and "flip" (left-right, upside down) for
// anything else.
//
// What goes wrong is said on standard error, in one line.

#include <libcamera/libcamera.h>

#include "photo.h"
#include "stack.h"
#include "still.h"

#include <fcntl.h>
#include <linux/dma-buf.h>
#include <sys/ioctl.h>
#include <sys/mman.h>
#include <sys/wait.h>
#include <sys/resource.h>
#include <sys/stat.h>
#include <sys/syscall.h>
#include <unistd.h>

#include <algorithm>
#include <atomic>
#include <cerrno>
#include <chrono>
#include <cmath>
#include <condition_variable>
#include <csetjmp>
#include <csignal>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <ctime>
#include <deque>
#include <fstream>
#include <iostream>
#include <map>
#include <memory>
#include <mutex>
#include <numeric>
#include <string>
#include <thread>
#include <vector>

using namespace libcamera;

namespace {

std::atomic<bool> running{true};
std::shared_ptr<Camera> camera;
bool colour = false;
Orientation wanted = Orientation::Rotate180;
unsigned int width = 640, height = 480;
unsigned int fps = 30, slowest = 15;
std::atomic<bool> night{false};

// Turned half round here, as each frame is copied, because the camera
// pipeline could not do it.
bool turn_here = false;

// The pro dials, as the shell last asked ("pro …"): 0 leaves each to the
// camera. Put into the next preview request when they change.
struct ProControls {
	int64_t exposure_us = 0;
	float gain = 0;
	float ev = 0;
	int ct = 0;
	int64_t max_us = 0;
	// White balance gains fixed (red, blue: the grey card's), else 0.
	float gains[2] = { 0, 0 };
	// Metered on the middle alone (the exposure lock), else centre-weighted.
	bool spot = false;
};
std::mutex pro_lock;
ProControls pro;
std::atomic<bool> pro_dirty{false};

// A stack under way told to end once it has enough (1), or at once with
// no photo (2); 0 while nothing is asked. Set as the order is read.
std::atomic<int> stack_order{0};

// The focus check: asked for, where (0 to 1, the picture's way up), and its
// scale once running (1: the full sensor one to one; 0.5: the binned mode;
// 0: the whole picture). Its crop goes into the next request when it moves.
std::atomic<bool> focus_on{false};
std::atomic<float> focus_x{0.5f}, focus_y{0.5f};
std::atomic<float> focus_zoom{0.0f};
std::atomic<bool> crop_dirty{false};
Rectangle crop_max;

// ---------------------------------------------------------------- output ---

// The whole of `n` bytes to standard output; false once the reader has gone.
bool send(const uint8_t *data, size_t n)
{
	while (n > 0) {
		ssize_t w = write(STDOUT_FILENO, data, n);
		if (w < 0) {
			if (errno == EINTR)
				continue;
			return false;
		}
		data += w;
		n -= size_t(w);
	}
	return true;
}

bool send_header(uint32_t kind, uint32_t length, uint32_t number)
{
	uint8_t head[16] = { 'C', 'C', 'A', 'M' };
	for (int i = 0; i < 4; i++) {
		head[4 + i] = uint8_t(kind >> (8 * i));
		head[8 + i] = uint8_t(length >> (8 * i));
		head[12 + i] = uint8_t(number >> (8 * i));
	}
	return send(head, sizeof head);
}

// The newest frame, copied out of the camera's buffer, and the count of
// frames copied so far; and the messages waiting. The writer takes them when
// it is ready.
std::mutex lock;
std::condition_variable fresh;
std::vector<uint8_t> newest;
std::string newest_said;
uint64_t made = 0;
struct Message {
	uint32_t kind, number;
	std::string text;
};
std::deque<Message> messages;

void tell(uint32_t kind, uint32_t number, const std::string &text)
{
	{
		std::lock_guard<std::mutex> hold(lock);
		messages.push_back({ kind, number, text });
	}
	fresh.notify_one();
}

// The writer: messages first, then the newest frame each time the reader is
// ready for one.
void write_frames()
{
	std::vector<uint8_t> sending;
	std::string said;
	uint64_t sent = 0;
	while (running) {
		std::deque<Message> say;
		bool frame = false;
		{
			std::unique_lock<std::mutex> hold(lock);
			fresh.wait_for(hold, std::chrono::milliseconds(100), [&] { return made != sent || !messages.empty() || !running; });
			if (!running)
				break;
			std::swap(say, messages);
			if (made != sent) {
				sending.resize(newest.size());
				std::swap(newest, sending);
				std::swap(newest_said, said);
				sent = made;
				frame = true;
			}
		}
		for (const Message &m : say) {
			if (!send_header(m.kind, uint32_t(m.text.size()), m.number) || !send(reinterpret_cast<const uint8_t *>(m.text.data()), m.text.size()))
				running = false;
		}
		// What the camera said of the frame, just before it.
		if (frame && running && colour && !said.empty()) {
			if (!send_header(5, uint32_t(said.size()), 0) || !send(reinterpret_cast<const uint8_t *>(said.data()), said.size()))
				running = false;
		}
		if (frame && running) {
			if (colour && !send_header(1, uint32_t(sending.size()), width << 16 | height))
				running = false;
			else if (!send(sending.data(), sending.size()))
				running = false;
		}
	}
	running = false;
	fresh.notify_all();
}

// ---------------------------------------------------------------- frames ---

// A buffer's planes, mapped: where Y, U and V start, and their strides.
struct Mapped {
	const uint8_t *y = nullptr, *u = nullptr, *v = nullptr;
};

// The camera running one configuration: its stream, buffers and requests.
// `raw`, the preview's only: the sensor's own frames, one in each request,
// so the pipeline (keeping a single raw frame of its own) never waits for a
// buffer to capture into.
struct Session {
	std::unique_ptr<CameraConfiguration> config;
	Stream *stream = nullptr;
	Stream *raw = nullptr;
	unsigned int w = 0, h = 0, stride = 0;
	std::unique_ptr<FrameBufferAllocator> allocator;
	std::map<FrameBuffer *, Mapped> planes;
	std::vector<std::pair<void *, size_t>> maps;
	std::vector<std::unique_ptr<Request>> requests;
	// A photo's RAW: the sensor's frames, mapped, and how they are laid out.
	std::map<FrameBuffer *, const uint8_t *> raw_planes;
	unsigned int raw_w = 0, raw_h = 0, raw_stride = 0;
	std::string raw_format;
	// The photo's frame held across photos (see Held), in place of one of
	// the allocator's: its dma-buf, to be synchronised before it is read.
	std::unique_ptr<FrameBuffer> external;
	int external_fd = -1;

	~Session() { close(); }

	void close()
	{
		requests.clear();
		for (auto &[base, length] : maps)
			munmap(base, length);
		maps.clear();
		planes.clear();
		raw_planes.clear();
		external.reset();
		external_fd = -1;
		if (allocator && stream)
			allocator->free(stream);
		if (allocator && raw)
			allocator->free(raw);
		allocator.reset();
		config.reset();
		stream = nullptr;
		raw = nullptr;
	}
};

// The photo's frame, held from the kernel's CMA heap while the camera is
// open (allocated at the first photo, if the heap gives it): the ISP writes
// each photo into it, so a photo asks the pool for 22 MB (the pipeline's
// own buffers), not 34, and never waits on it for this one. Given up for
// good if anything about it fails, and photos take their frame from the
// allocator as before.
struct Held {
	int fd = -1;
	size_t size = 0;
	uint8_t *map = nullptr;
	bool failed = false;

	~Held() { release(); }

	void release()
	{
		if (map)
			munmap(map, size);
		if (fd >= 0)
			::close(fd);
		map = nullptr;
		fd = -1;
		size = 0;
	}

	// Given up for good: for this program, and (its mark in /run) for the
	// rest of this boot, so the next Camera does not try it again first.
	void give_up(const std::string &why)
	{
		if (!failed)
			std::fprintf(stderr, "still: the held frame failed (%s); photos take the allocator's from now on\n", why.c_str());
		failed = true;
		release();
		int mark = open(OFF_MARK, O_WRONLY | O_CREAT | O_CLOEXEC, 0644);
		if (mark >= 0) {
			ssize_t ignored = write(mark, why.data(), why.size());
			(void)ignored;
			::close(mark);
		}
	}

	static constexpr const char *OFF_MARK = "/run/catalyst-camera.held-off";

	// At least `want` bytes of it; false if it cannot be had.
	bool get(size_t want)
	{
		if (!failed && access(OFF_MARK, F_OK) == 0)
			failed = true;
		if (failed)
			return false;
		if (fd >= 0 && size >= want)
			return true;
		release();
		// The kernel's dma-heap ioctl (linux/dma-heap.h, newer than this
		// toolchain's headers).
		struct Allocation {
			uint64_t len;
			uint32_t fd;
			uint32_t fd_flags;
			uint64_t heap_flags;
		} allocation = { want, 0, O_RDWR | O_CLOEXEC, 0 };
		int heap = open("/dev/dma_heap/linux,cma", O_RDWR | O_CLOEXEC);
		if (heap < 0) {
			failed = true;
			return false;
		}
		int r = ioctl(heap, _IOWR('H', 0x0, Allocation), &allocation);
		::close(heap);
		if (r < 0) {
			failed = true;
			return false;
		}
		fd = int(allocation.fd);
		size = want;
		void *at = mmap(nullptr, want, PROT_READ, MAP_SHARED, fd, 0);
		if (at == MAP_FAILED) {
			release();
			failed = true;
			return false;
		}
		map = static_cast<uint8_t *>(at);
		return true;
	}
};
Held held;

// A dma-buf's caches made right for the processor to read it (begin) or
// done with (end): the ISP wrote it behind the caches' back.
void dma_sync(int fd, bool begin)
{
	if (fd < 0)
		return;
	struct dma_buf_sync sync = {};
	sync.flags = (begin ? DMA_BUF_SYNC_START : DMA_BUF_SYNC_END) | DMA_BUF_SYNC_READ;
	while (ioctl(fd, DMA_BUF_IOCTL_SYNC, &sync) < 0 && errno == EINTR) {
	}
}

// Rows of `w` bytes from a plane with rows `stride` apart into `out`,
// packed, turned half round if asked.
void copy_plane(const uint8_t *plane, unsigned int stride, unsigned int w, unsigned int h, uint8_t *out, bool turned)
{
	for (unsigned int y = 0; y < h; y++) {
		const uint8_t *row = plane + size_t(y) * stride;
		if (turned)
			std::reverse_copy(row, row + w, out + size_t(h - 1 - y) * w);
		else
			std::memcpy(out + size_t(y) * w, row, w);
	}
}

// A `W` x `H` frame's crop `c` (the right way up) into `out`, packed: Y,
// then U, then V. Turned half round here, the crop's place in the buffer
// is the opposite corner's.
void copy_frame(const Mapped &m, unsigned int stride, unsigned int W, unsigned int H, still::Crop c, bool turned, uint8_t *out)
{
	unsigned int x = turned ? W - c.x - c.w : c.x, y = turned ? H - c.y - c.h : c.y;
	copy_plane(m.y + size_t(y) * stride + x, stride, c.w, c.h, out, turned);
	const size_t cs = stride / 2, at = size_t(y / 2) * cs + x / 2;
	copy_plane(m.u + at, unsigned(cs), c.w / 2, c.h / 2, out + size_t(c.w) * c.h, turned);
	copy_plane(m.v + at, unsigned(cs), c.w / 2, c.h / 2, out + size_t(c.w) * c.h * 5 / 4, turned);
}

// What the camera thread does with a finished request.
enum class Phase { Preview, Still, Burst, Stopped };

// Detail mode's burst: each frame handed to the main thread as it comes.
std::mutex burst_lock;
std::condition_variable burst_ready;
std::deque<Request *> burst_done;
// A stack's order has come (under burst_lock): its wait looks up.
bool stack_woken = false;

// The light as the preview last saw it (lux, and the gain it took), for
// detail mode's "auto": on in dim light.
std::atomic<float> seen_lux{ 400.0f };
std::atomic<float> seen_gain{ 1.0f };
std::atomic<Phase> phase{Phase::Stopped};
// The session running, read on the camera's thread as the main thread
// changes it (a photo stops the preview, takes a still, starts it again).
std::atomic<Session *> current{nullptr};

// A photo being taken: each frame judged as it comes (still.h), and the
// request holding the one kept, or why none will come this way.
std::mutex still_lock;
std::condition_variable still_ready;
still::Judge still_judge;
Request *still_request = nullptr;
std::string still_failed;
// The photo's exposure when the shell fixed it (µs), else 0.
int64_t still_fixed_us = 0;

// ---------------------------------------------------------- face metering ---

// A face the smart camera on the Grove port sees, mapped by the shell into
// this camera's picture (0 to 1 each way, the picture's way up): the
// exposure leans toward it. Every few frames its brightness is measured in
// the preview and the exposure compensation the camera's own AE follows is
// stepped toward what brings it to a face's usual level; with no face for a
// moment it eases back to none. A photo starts from where it got to.
std::mutex face_lock;
bool face_on = false;
float face_box[4] = { 0, 0, 0, 0 };
std::chrono::steady_clock::time_point face_seen;
// The compensation in force (EV), and the frames since it was last weighed:
// the camera's thread's own, read by the main one only while it is stopped.
float ev = 0.0f;
float ev_sent = 0.0f;
int since_weighed = 0;
// A face well exposed: its mean brightness, 0 to 1 as encoded (about 120).
const float FACE_LEVEL = 0.47f;
const float EV_LOW = -1.5f, EV_HIGH = 2.0f;

// True when the compensation is to change (with the request requeued).
bool lean_on_face(const Mapped &m, unsigned int w, unsigned int h, unsigned int stride)
{
	if (++since_weighed < 3)
		return false;
	since_weighed = 0;
	bool on;
	float box[4];
	{
		std::lock_guard<std::mutex> hold(face_lock);
		on = face_on && std::chrono::steady_clock::now() - face_seen < std::chrono::milliseconds(800);
		std::copy(face_box, face_box + 4, box);
	}
	if (on) {
		// The box in the buffer's rows (turned back if the turn is done here).
		if (turn_here)
			box[0] = 1 - box[0], box[2] = 1 - box[2], box[1] = 1 - box[1], box[3] = 1 - box[3];
		unsigned int x0 = unsigned(std::clamp(std::min(box[0], box[2]), 0.0f, 1.0f) * w);
		unsigned int x1 = unsigned(std::clamp(std::max(box[0], box[2]), 0.0f, 1.0f) * w);
		unsigned int y0 = unsigned(std::clamp(std::min(box[1], box[3]), 0.0f, 1.0f) * h);
		unsigned int y1 = unsigned(std::clamp(std::max(box[1], box[3]), 0.0f, 1.0f) * h);
		uint64_t sum = 0, n = 0;
		for (unsigned int y = y0; y < y1; y += 2) {
			const uint8_t *row = m.y + size_t(y) * stride;
			for (unsigned int x = x0; x < x1; x += 2)
				sum += row[x], n++;
		}
		if (n >= 16) {
			float level = std::max(float(sum) / float(n) / 255.0f, 0.02f);
			// Encoded brightness is about linear light to the 1/2.2.
			float step = 2.2f * std::log2(FACE_LEVEL / level);
			ev = std::clamp(ev + 0.3f * step, EV_LOW, EV_HIGH);
		}
	} else {
		ev *= 0.85f;
		if (std::fabs(ev) < 0.02f)
			ev = 0.0f;
	}
	if (std::fabs(ev - ev_sent) >= 0.05f || (ev == 0.0f && ev_sent != 0.0f)) {
		ev_sent = ev;
		return true;
	}
	return false;
}

// "face x0 y0 x1 y1" (0 to 1 in this camera's picture) or "face none".
void take_face(const std::string &order)
{
	std::lock_guard<std::mutex> hold(face_lock);
	float b[4];
	if (std::sscanf(order.c_str(), "face %f %f %f %f", &b[0], &b[1], &b[2], &b[3]) == 4) {
		std::copy(b, b + 4, face_box);
		face_on = true;
		face_seen = std::chrono::steady_clock::now();
	} else {
		face_on = false;
	}
}

// The preview's longest frame: the pro dials' (a long shutter shown at a
// frame's length, the gain making up the rest: pro.rs), night's, or the
// slowest frame rate's.
int64_t preview_longest()
{
	int64_t asked;
	{
		std::lock_guard<std::mutex> hold(pro_lock);
		asked = pro.max_us;
	}
	if (asked > 0)
		return std::clamp<int64_t>(asked, 1000000 / fps, 1000000);
	return night ? 200000 : int64_t(1000000 / slowest);
}

float pro_ev()
{
	std::lock_guard<std::mutex> hold(pro_lock);
	return pro.ev;
}

// The pro dials into a preview request's controls (and the frame limits
// with them): fixed where set, the camera's own where not.
void pro_into(ControlList &c, bool with_limits)
{
	ProControls p;
	{
		std::lock_guard<std::mutex> hold(pro_lock);
		p = pro;
	}
	if (p.exposure_us > 0) {
		c.set(controls::ExposureTimeMode, controls::ExposureTimeModeManual);
		c.set(controls::ExposureTime, int32_t(p.exposure_us));
	} else {
		c.set(controls::ExposureTimeMode, controls::ExposureTimeModeAuto);
	}
	if (p.gain > 0) {
		c.set(controls::AnalogueGainMode, controls::AnalogueGainModeManual);
		c.set(controls::AnalogueGain, p.gain);
	} else {
		c.set(controls::AnalogueGainMode, controls::AnalogueGainModeAuto);
	}
	// The face's lean rides on the compensation asked for.
	c.set(controls::ExposureValue, p.ev + ev);
	c.set(controls::AeMeteringMode, p.spot ? controls::MeteringSpot : controls::MeteringCentreWeighted);
	if (p.gains[0] > 0 && p.gains[1] > 0) {
		c.set(controls::AwbEnable, false);
		c.set(controls::ColourGains, Span<const float, 2>({ p.gains[0], p.gains[1] }));
	} else if (p.ct > 0) {
		c.set(controls::ColourTemperature, int32_t(p.ct));
	} else {
		c.set(controls::AwbEnable, true);
	}
	if (with_limits) {
		int64_t longest = std::max<int64_t>(preview_longest(), p.exposure_us);
		c.set(controls::FrameDurationLimits, { int64_t(1000000 / fps), longest });
	}
}

// The focus check's crop in the sensor's own pixels: the preview's size at
// one sensor pixel a preview pixel (two in the binned mode), centred where
// asked as far as the picture's edges allow. A turn done here (not by the
// sensor) turns the crop's place round too.
Rectangle focus_crop()
{
	const Rectangle &max = crop_max;
	if (max.width == 0 || max.height == 0)
		return max;
	unsigned int k = focus_zoom.load() >= 0.99f ? 1 : 2;
	unsigned int cw = std::min(width * k, max.width), ch = std::min(height * k, max.height);
	float x = focus_x.load(), y = focus_y.load();
	if (turn_here) {
		x = 1 - x;
		y = 1 - y;
	}
	int cx = max.x + int(x * float(max.width)), cy = max.y + int(y * float(max.height));
	int left = std::clamp(cx - int(cw / 2), max.x, max.x + int(max.width - cw));
	int top = std::clamp(cy - int(ch / 2), max.y, max.y + int(max.height - ch));
	return Rectangle(left, top, cw, ch);
}

// A frame's state, as still.h judges it.
still::Status status_of(Request *request, Stream *stream)
{
	if (request->status() == Request::RequestCancelled)
		return still::Cancelled;
	FrameBuffer *buffer = request->findBuffer(stream);
	if (!buffer)
		return still::Error;
	switch (buffer->metadata().status) {
	case FrameMetadata::FrameSuccess:
		return still::Success;
	case FrameMetadata::FrameStartup:
		return still::Startup;
	default:
		return still::Error;
	}
}

// A photo's frame, judged: kept, let go (the request straight back for the
// next), or the try given up at once (a refused request is cancelled, and
// nothing more would ever come of it: 0.9.3 waited six seconds for one).
void judge_still(Request *request, Session *session)
{
	std::lock_guard<std::mutex> hold(still_lock);
	if (still_request || !still_failed.empty())
		return;
	still::Status status = status_of(request, session->stream);
	bool matches = false, converged = false;
	if (status != still::Cancelled) {
		const ControlList &said = request->metadata();
		matches = still::matches(said.get(controls::ExposureTime).value_or(0), still_fixed_us);
		auto ae = said.get(controls::AeState);
		converged = ae && *ae == controls::AeStateConverged;
	}
	bool long_exposure = still_fixed_us > 1000000;
	switch (still_judge.see(status, still_fixed_us > 0, long_exposure, matches, converged)) {
	case still::Take:
		still_request = request;
		still_ready.notify_one();
		return;
	case still::Fail:
		still_failed = still_judge.why;
		still_ready.notify_one();
		return;
	case still::Wait:
		break;
	}
	request->reuse(Request::ReuseBuffers);
	if (camera->queueRequest(request) < 0) {
		still_failed = "request refused";
		still_ready.notify_one();
	}
}

// On the camera's thread: copy the frame and give the buffer straight back
// (a photo's frame is kept until it is copied out).
void completed(Request *request)
{
	Session *session = current.load();
	if (!running || !session)
		return;
	if (phase == Phase::Still) {
		judge_still(request, session);
		return;
	}
	// A burst's or a bracketing set's frames, judged on the main thread
	// (a cancelled one too: it ends them).
	if (phase == Phase::Burst) {
		{
			std::lock_guard<std::mutex> hold(burst_lock);
			burst_done.push_back(request);
		}
		burst_ready.notify_one();
		return;
	}
	if (request->status() == Request::RequestCancelled)
		return;
	FrameBuffer *buffer = request->findBuffer(session->stream);
	if (!buffer)
		return;
	bool good = buffer->metadata().status == FrameMetadata::FrameSuccess;
	if (phase != Phase::Preview)
		return;
	// What the camera said of the frame, for the shell (kind 5).
	std::string said_line;
	{
		const ControlList &said = request->metadata();
		if (auto lux = said.get(controls::Lux))
			seen_lux = *lux;
		float ag = said.get(controls::AnalogueGain).value_or(1.0f), dg = said.get(controls::DigitalGain).value_or(1.0f);
		seen_gain = ag * dg;
		if (colour) {
			float r = 0, b = 0;
			if (auto gains = said.get(controls::ColourGains)) {
				r = (*gains)[0];
				b = (*gains)[1];
			}
			auto ae = said.get(controls::AeState);
			bool settled = !ae || *ae != controls::AeStateSearching;
			char line[200];
			std::snprintf(line, sizeof line, "exp=%d ag=%.3f dg=%.3f r=%.4f b=%.4f ct=%d lux=%.1f ae=%d fom=%d zoom=%.2f", int(said.get(controls::ExposureTime).value_or(0)), ag, dg, r, b, int(said.get(controls::ColourTemperature).value_or(0)), double(said.get(controls::Lux).value_or(0.0f)), settled ? 1 : 0, int(said.get(controls::FocusFoM).value_or(0)), double(focus_zoom.load()));
			said_line = line;
		}
	}
	auto plane = session->planes.find(buffer);
	if (plane != session->planes.end() && good) {
		{
			std::lock_guard<std::mutex> hold(lock);
			const Mapped &m = plane->second;
			unsigned int w = session->w, h = session->h, s = session->stride;
			if (colour) {
				newest.resize(size_t(w) * h * 3 / 2);
				uint8_t *out = newest.data();
				copy_plane(m.y, s, w, h, out, turn_here);
				copy_plane(m.u, s / 2, w / 2, h / 2, out + size_t(w) * h, turn_here);
				copy_plane(m.v, s / 2, w / 2, h / 2, out + size_t(w) * h * 5 / 4, turn_here);
				newest_said = std::move(said_line);
			} else {
				newest.resize(size_t(w) * h);
				copy_plane(m.y, s, w, h, newest.data(), turn_here);
			}
			made++;
		}
		fresh.notify_one();
		bool lean = lean_on_face(plane->second, session->w, session->h, session->stride);
		request->reuse(Request::ReuseBuffers);
		// The dials changed: all of them into this request; else the face's lean.
		if (pro_dirty.exchange(false))
			pro_into(request->controls(), true);
		else if (lean)
			request->controls().set(controls::ExposureValue, pro_ev() + ev);
		if (crop_dirty.exchange(false) && focus_zoom.load() > 0.0f)
			request->controls().set(controls::ScalerCrop, focus_crop());
		camera->queueRequest(request);
		return;
	}
	request->reuse(Request::ReuseBuffers);
	camera->queueRequest(request);
}

// Which of the sensor's modes: libcamera's choice, the full view binned
// (1640 × 1232, the preview's), or every pixel (3280 × 2464, the focus
// check's however small its output).
enum class Sensor { Any, Binned, Full };

// What a session is opened with beyond its stream.
struct Opening {
	Sensor sensor = Sensor::Any;
	// The sensor's frames in each request too (the preview: see Session).
	bool raw = false;
	// A photo's: its raw frames mapped to be read (RAW), its frame the held one.
	bool map_raw = false;
	bool use_held = false;
};

// The camera set up for `role` at `size`, in YUV 4:2:0, turned as wanted.
// Empty with `why` said when it cannot be.
std::unique_ptr<Session> open_session(StreamRole role, Size size, unsigned int buffers, Opening how, std::string &why)
{
	const bool binned = how.sensor == Sensor::Binned;
	bool raw = how.raw;
	auto s = std::make_unique<Session>();
	std::vector<StreamRole> roles = { role };
	if (raw)
		roles.push_back(StreamRole::Raw);
	s->config = camera->generateConfiguration(roles);
	if ((!s->config || s->config->size() != roles.size()) && raw) {
		roles.pop_back();
		raw = false;
		s->config = camera->generateConfiguration(roles);
	}
	if (!s->config || s->config->empty()) {
		why = "the camera offers no picture";
		return nullptr;
	}
	s->config->orientation = wanted;
	StreamConfiguration &sc = s->config->at(0);
	sc.pixelFormat = formats::YUV420;
	sc.size = size;
	sc.bufferCount = buffers;
	// Colour as JPEG has it (full-range BT.601): photos are written from the
	// buffer as it is, and the shell turns the preview to RGB by one rule.
	if (colour)
		sc.colorSpace = ColorSpace::Sycc;
	if (raw) {
		StreamConfiguration &rc = s->config->at(1);
		rc.size = binned ? Size(1640, 1232) : rc.size;
		rc.bufferCount = buffers;
	}
	if (how.sensor != Sensor::Any) {
		// The full view, binned (what the lens figures are worked out for),
		// or every pixel of it.
		SensorConfiguration sensor;
		sensor.bitDepth = 10;
		sensor.analogCrop = Rectangle(0, 0, 3280, 2464);
		sensor.outputSize = binned ? Size(1640, 1232) : Size(3280, 2464);
		s->config->sensorConfig = sensor;
	}
	if (s->config->validate() == CameraConfiguration::Invalid) {
		// A raw stream it will not match: the same without one.
		if (raw) {
			how.raw = how.map_raw = false;
			return open_session(role, size, buffers, how, why);
		}
		// The full sensor asked for and not to be had: no.
		if (how.sensor == Sensor::Full) {
			why = "the camera cannot crop its full sensor";
			return nullptr;
		}
		// Another sensor: let libcamera choose its mode.
		s->config->sensorConfig.reset();
		s->config->orientation = wanted;
		if (s->config->validate() == CameraConfiguration::Invalid) {
			why = "the camera cannot give " + size.toString();
			return nullptr;
		}
	}
	if (sc.size != size || sc.pixelFormat != formats::YUV420) {
		why = "the camera gives " + sc.toString() + ", not " + size.toString() + " YUV420";
		return nullptr;
	}
	// What the pipeline could not turn, done here: half a turn is the only
	// one that keeps the frame's shape and costs no more than the copy.
	if (s->config->orientation != wanted) {
		if (wanted / s->config->orientation == Transform::Rot180) {
			turn_here = true;
		} else {
			why = "the camera cannot be turned that way";
			return nullptr;
		}
	} else {
		turn_here = false;
	}
	if (camera->configure(s->config.get())) {
		why = "the camera would not take its settings";
		return nullptr;
	}
	s->stream = sc.stream();
	s->raw = raw ? s->config->at(1).stream() : nullptr;
	s->w = sc.size.width;
	s->h = sc.size.height;
	s->stride = sc.stride;
	s->allocator = std::make_unique<FrameBufferAllocator>(camera);
	// A photo's frame: the held one, imported, when it can be had.
	const size_t ysize = size_t(s->stride) * s->h, csize = size_t(s->stride / 2) * (s->h / 2);
	if (how.use_held && buffers == 1 && held.get(std::max<size_t>(sc.frameSize, ysize + 2 * csize))) {
		std::vector<FrameBuffer::Plane> planes(3);
		const size_t offsets[3] = { 0, ysize, ysize + csize }, lengths[3] = { ysize, csize, csize };
		for (int i = 0; i < 3; i++) {
			planes[i].fd = SharedFD(held.fd);
			planes[i].offset = unsigned(offsets[i]);
			planes[i].length = unsigned(lengths[i]);
		}
		s->external = std::make_unique<FrameBuffer>(planes);
		s->external_fd = held.fd;
	} else if (s->allocator->allocate(s->stream) < 0) {
		why = "no memory for the camera's frames";
		return nullptr;
	}
	if (s->raw && s->allocator->allocate(s->raw) < 0) {
		why = "no memory for the camera's frames";
		return nullptr;
	}
	if (s->raw) {
		const StreamConfiguration &rc = s->config->at(1);
		s->raw_w = rc.size.width;
		s->raw_h = rc.size.height;
		s->raw_stride = rc.stride;
		s->raw_format = rc.pixelFormat.toString();
	}
	// The sensor's frames mapped to be read, for a RAW.
	if (s->raw && how.map_raw) {
		for (const std::unique_ptr<FrameBuffer> &buffer : s->allocator->buffers(s->raw)) {
			const FrameBuffer::Plane &p = buffer->planes()[0];
			size_t length = size_t(p.offset) + p.length;
			void *at = mmap(nullptr, length, PROT_READ, MAP_SHARED, p.fd.get(), 0);
			if (at == MAP_FAILED) {
				why = "the camera's raw frames cannot be read";
				return nullptr;
			}
			s->maps.emplace_back(at, length);
			s->raw_planes[buffer.get()] = static_cast<const uint8_t *>(at) + p.offset;
		}
	}
	const auto &raws = s->raw ? s->allocator->buffers(s->raw) : s->allocator->buffers(s->stream);
	if (s->external) {
		Mapped m;
		m.y = held.map;
		m.u = m.y + ysize;
		m.v = m.u + csize;
		s->planes[s->external.get()] = m;
		std::unique_ptr<Request> request = camera->createRequest();
		if (!request || request->addBuffer(s->stream, s->external.get()) || (s->raw && !raws.empty() && request->addBuffer(s->raw, raws[0].get()))) {
			why = "the camera would not take a request";
			return nullptr;
		}
		s->requests.push_back(std::move(request));
		return s;
	}
	size_t index = 0;
	for (const std::unique_ptr<FrameBuffer> &buffer : s->allocator->buffers(s->stream)) {
		// Each plane's file mapped once, as far as its furthest plane reaches.
		std::map<int, size_t> reach;
		for (const FrameBuffer::Plane &p : buffer->planes())
			reach[p.fd.get()] = std::max(reach[p.fd.get()], size_t(p.offset) + p.length);
		std::map<int, const uint8_t *> base;
		for (auto [fd, length] : reach) {
			void *at = mmap(nullptr, length, PROT_READ, MAP_SHARED, fd, 0);
			if (at == MAP_FAILED) {
				why = "the camera's frames cannot be read";
				return nullptr;
			}
			s->maps.emplace_back(at, length);
			base[fd] = static_cast<const uint8_t *>(at);
		}
		const auto &p = buffer->planes();
		Mapped m;
		m.y = base[p[0].fd.get()] + p[0].offset;
		if (p.size() >= 3) {
			m.u = base[p[1].fd.get()] + p[1].offset;
			m.v = base[p[2].fd.get()] + p[2].offset;
		} else {
			// One plane holding all three, packed one after another.
			m.u = m.y + size_t(s->stride) * s->h;
			m.v = m.u + size_t(s->stride / 2) * (s->h / 2);
		}
		s->planes[buffer.get()] = m;
		std::unique_ptr<Request> request = camera->createRequest();
		if (!request || request->addBuffer(s->stream, buffer.get())) {
			why = "the camera would not take a request";
			return nullptr;
		}
		if (s->raw && index < raws.size() && request->addBuffer(s->raw, raws[index].get())) {
			why = "the camera would not take a request";
			return nullptr;
		}
		index++;
		s->requests.push_back(std::move(request));
	}
	return s;
}

// The frame duration limits: as fast as `fastest` frames a second, as slow
// as the light needs within `longest_us` of exposure.
ControlList limits(unsigned int fastest, int64_t longest_us)
{
	ControlList controls;
	controls.set(controls::FrameDurationLimits, { int64_t(1000000 / fastest), longest_us });
	return controls;
}

void stop_camera();

bool start(Session &s, ControlList controls, Phase as, std::string &why)
{
	current.store(&s);
	phase = as;
	if (camera->start(&controls)) {
		why = "the camera would not start";
		phase = Phase::Stopped;
		current.store(nullptr);
		return false;
	}
	// A request refused here would never come back: said now, not waited on.
	for (std::unique_ptr<Request> &request : s.requests) {
		int r = camera->queueRequest(request.get());
		if (r < 0) {
			why = "the camera refused a request (" + std::string(std::strerror(-r)) + ")";
			stop_camera();
			return false;
		}
	}
	return true;
}

void stop_camera()
{
	phase = Phase::Stopped;
	camera->stop();
	current.store(nullptr);
}

std::unique_ptr<Session> preview;

bool start_preview(std::string &why)
{
	preview.reset();
	focus_zoom = 0.0f;
	if (focus_on && colour) {
		// The focus check: every pixel of the sensor, cropped to the
		// preview's size (no raw frames of ours: the full sensor's are 10 MB
		// each); where that will not run, the binned mode cropped.
		Opening full;
		full.sensor = Sensor::Full;
		preview = open_session(StreamRole::VideoRecording, Size(width, height), 4, full, why);
		if (preview)
			focus_zoom = 1.0f;
	}
	if (!preview) {
		Opening binned;
		binned.sensor = Sensor::Binned;
		binned.raw = true;
		preview = open_session(StreamRole::VideoRecording, Size(width, height), 4, binned, why);
		if (preview && focus_on && colour)
			focus_zoom = 0.5f;
	}
	if (!preview)
		return false;
	// The crop's limits in this mode.
	crop_max = Rectangle();
	auto info = camera->controls().find(&controls::ScalerCrop);
	if (info != camera->controls().end())
		crop_max = info->second.max().get<Rectangle>();
	else if (auto most = camera->properties().get(properties::ScalerCropMaximum))
		crop_max = *most;
	// The exposure and white balance as the dials have them (free in auto:
	// a burst holds them).
	ControlList controls;
	pro_into(controls, true);
	if (focus_zoom > 0.0f && crop_max.width > 0)
		controls.set(controls::ScalerCrop, focus_crop());
	pro_dirty = false;
	crop_dirty = false;
	return start(*preview, std::move(controls), Phase::Preview, why);
}

// ------------------------------------------------------------------ shot ---

// What a photo is taken with, from the words after its path (see the notes
// at the top); each left as the camera has it when not given.
struct Asked {
	int64_t exposure_us = 0;
	float gain = 0;
	float gains[2] = { 0, 0 };
	int ct = 0;
	float sharpness = 0;
	std::string denoise;
	std::string look, look_name, original, raw;
	// The aspect's crop (0:0 the whole frame), the compensation and the
	// exposure program for Exif, and a bracketing set's other frames.
	unsigned aspect_a = 0, aspect_b = 0;
	int bias_thirds = 0;
	int program = 2;
	struct Also {
		int64_t exposure_us = 0;
		float gain = 1.0f;
		int bias_thirds = 0;
		std::string path;
	};
	std::vector<Also> also;
	// Night and HDR: the frames to merge (0 or 1: none), what for, and the
	// finish (any photo may have one).
	int stack = 0;
	std::string stack_for;
	stack::Finish finish;
};

Asked asked_of(const std::string &words)
{
	Asked a;
	size_t at = 0;
	while (at < words.size()) {
		size_t end = words.find('\t', at);
		std::string word = words.substr(at, end == std::string::npos ? std::string::npos : end - at);
		at = end == std::string::npos ? words.size() : end + 1;
		size_t eq = word.find('=');
		if (eq == std::string::npos)
			continue;
		std::string key = word.substr(0, eq), value = word.substr(eq + 1);
		if (key == "exp")
			a.exposure_us = std::strtoll(value.c_str(), nullptr, 10);
		else if (key == "gain")
			a.gain = std::strtof(value.c_str(), nullptr);
		else if (key == "gains")
			std::sscanf(value.c_str(), "%f,%f", &a.gains[0], &a.gains[1]);
		else if (key == "ct")
			a.ct = std::atoi(value.c_str());
		else if (key == "sharp")
			a.sharpness = std::strtof(value.c_str(), nullptr);
		else if (key == "nr")
			a.denoise = value;
		else if (key == "look")
			a.look = value;
		else if (key == "lookname")
			a.look_name = value;
		else if (key == "orig")
			a.original = value;
		else if (key == "raw")
			a.raw = value;
		else if (key == "aspect")
			still::read_aspect(value, a.aspect_a, a.aspect_b);
		else if (key == "bias")
			a.bias_thirds = std::atoi(value.c_str());
		else if (key == "program")
			a.program = std::clamp(std::atoi(value.c_str()), 0, 8);
		else if (key == "stack")
			a.stack = std::clamp(std::atoi(value.c_str()), 0, still::STACK_MOST);
		else if (key == "stackfor")
			a.stack_for = value == "hdr" ? "hdr" : "night";
		else if (key == "lift")
			a.finish.lift = std::clamp(std::strtof(value.c_str(), nullptr), 0.0f, 4.0f);
		else if (key == "toe")
			a.finish.toe = std::clamp(std::strtof(value.c_str(), nullptr), 0.0f, 0.5f);
		else if (key == "neutral")
			a.finish.neutral = std::clamp(std::strtof(value.c_str(), nullptr), 0.0f, 1.0f);
		else if (key == "also") {
			Asked::Also more;
			int used = 0;
			long long e = 0;
			if (std::sscanf(value.c_str(), "%lld,%f,%d,%n", &e, &more.gain, &more.bias_thirds, &used) >= 3 && used > 0 && e > 0) {
				more.exposure_us = std::min<int64_t>(e, still::LONGEST_US);
				more.path = value.substr(size_t(used));
				if (!more.path.empty() && a.also.size() < 4)
					a.also.push_back(more);
			}
		}
	}
	return a;
}

// A photo's picture, copied out of the camera's memory (packed planes, Y
// then U then V, the right way up), and what the camera said of it.
struct Shot {
	std::string path;
	unsigned int w = 0, h = 0;
	std::vector<uint8_t> yuv;
	int64_t exposure_us = 0;
	float gain = 1.0f;
	float gains[2] = { 0, 0 };
	std::time_t at = 0;
	std::chrono::steady_clock::time_point asked;
	long shot_ms = 0;
	// Detail mode: the burst's other frames (memory files), and the face
	// the smart camera saw, if any, for the merge.
	std::vector<int> burst;
	bool face = false;
	float face_box[4] = { 0, 0, 0, 0 };
	// The look over it, the photo as taken kept beside it, the RAW (its
	// frame in a memory file, as the sensor gave it) and how it went.
	Asked with;
	int raw_fd = -1;
	unsigned int raw_w = 0, raw_h = 0, raw_stride = 0;
	std::string raw_format;
	int black = 4096;
	bool raw_skipped = false;
	// Its compensation (Exif), and its timing marks.
	int bias_thirds = 0;
	still::Marks marks;
	// A stack: the frames merged of those asked, each frame's exposure
	// (`exposure_us` is then all of them together, as its Exif says).
	int stacked = 0, stack_planned = 0;
	int64_t frame_us = 0;
};

// ------------------------------------------------------------------ exif ---

// What the photos say of their lens: the IMX219's, as the AprilTag reader
// takes it (the Raspberry Pi camera's 3.04 mm at f/2.0; 29 mm in 35 mm terms
// on its 4.6 mm diagonal).
const char *LENS = "IMX219 3.04mm f/2.0 (main camera)";

std::string software()
{
	std::ifstream version("/etc/catalyst/version");
	std::string line;
	while (std::getline(version, line))
		if (line.rfind("version=", 0) == 0)
			return "Catalyst Linux " + line.substr(8);
	return "Catalyst Linux";
}

std::vector<uint8_t> le32(uint32_t v)
{
	return { uint8_t(v), uint8_t(v >> 8), uint8_t(v >> 16), uint8_t(v >> 24) };
}

// One TIFF directory, little-endian, as Exif has it.
struct Ifd {
	struct Entry {
		uint16_t tag, type;
		uint32_t count;
		std::vector<uint8_t> data;
	};
	std::vector<Entry> entries;

	void add(uint16_t tag, uint16_t type, uint32_t count, std::vector<uint8_t> data) { entries.push_back({ tag, type, count, std::move(data) }); }
	void ascii(uint16_t tag, const std::string &s)
	{
		std::vector<uint8_t> d(s.begin(), s.end());
		d.push_back(0);
		uint32_t n = uint32_t(d.size());
		add(tag, 2, n, std::move(d));
	}
	void shorts(uint16_t tag, uint16_t v) { add(tag, 3, 1, { uint8_t(v), uint8_t(v >> 8) }); }
	void longs(uint16_t tag, uint32_t v) { add(tag, 4, 1, le32(v)); }
	void rational(uint16_t tag, uint32_t n, uint32_t d)
	{
		std::vector<uint8_t> v = le32(n), b = le32(d);
		v.insert(v.end(), b.begin(), b.end());
		add(tag, 5, 1, std::move(v));
	}
	void undefined(uint16_t tag, const std::vector<uint8_t> &four) { add(tag, 7, uint32_t(four.size()), four); }

	// Its length: the entries, then each value longer than four bytes.
	size_t size() const
	{
		size_t n = 2 + 12 * entries.size() + 4;
		for (const Entry &e : entries)
			if (e.data.size() > 4)
				n += (e.data.size() + 1) & ~size_t(1);
		return n;
	}

	// Written at `offset` from the TIFF header, its entries in tag order.
	void write(std::vector<uint8_t> &out, uint32_t offset)
	{
		std::sort(entries.begin(), entries.end(), [](const Entry &a, const Entry &b) { return a.tag < b.tag; });
		auto put = [&](uint32_t v, int bytes) {
			for (int i = 0; i < bytes; i++)
				out.push_back(uint8_t(v >> (8 * i)));
		};
		uint32_t extra = offset + uint32_t(2 + 12 * entries.size() + 4);
		put(uint32_t(entries.size()), 2);
		for (const Entry &e : entries) {
			put(e.tag, 2);
			put(e.type, 2);
			put(e.count, 4);
			if (e.data.size() <= 4) {
				for (size_t i = 0; i < 4; i++)
					out.push_back(i < e.data.size() ? e.data[i] : 0);
			} else {
				put(extra, 4);
				extra += uint32_t((e.data.size() + 1) & ~size_t(1));
			}
		}
		put(0, 4);
		for (const Entry &e : entries) {
			if (e.data.size() <= 4)
				continue;
			out.insert(out.end(), e.data.begin(), e.data.end());
			if (e.data.size() & 1)
				out.push_back(0);
		}
	}
};

// The photo's Exif (APP1): the camera and its lens, the exposure as taken,
// the time with its offset from UTC, the picture's size.
std::vector<uint8_t> exif(const Shot &s)
{
	std::tm local{};
	localtime_r(&s.at, &local);
	char when[20], z[8], zone[8] = "+00:00";
	std::strftime(when, sizeof when, "%Y:%m:%d %H:%M:%S", &local);
	if (std::strftime(z, sizeof z, "%z", &local) == 5)
		std::snprintf(zone, sizeof zone, "%.3s:%.2s", z, z + 3);
	Ifd zero, sub;
	zero.ascii(0x010f, "M5Stack");
	zero.ascii(0x0110, "CardputerZero");
	zero.shorts(0x0112, 1);
	zero.rational(0x011a, 72, 1);
	zero.rational(0x011b, 72, 1);
	zero.shorts(0x0128, 2);
	zero.ascii(0x0131, software());
	zero.ascii(0x0132, when);
	// A stack (Night, HDR): what it was made of. Its exposure time below is
	// all of its frames' together.
	if (s.stacked > 1) {
		auto seconds = [](int64_t us) {
			char t[24];
			if (us >= 300000)
				std::snprintf(t, sizeof t, "%.1f", double(us) / 1e6);
			else
				std::snprintf(t, sizeof t, "1/%d", int(std::lround(1e6 / double(std::max<int64_t>(us, 1)))));
			std::string out = t;
			if (out.size() > 2 && out.compare(out.size() - 2, 2, ".0") == 0)
				out.resize(out.size() - 2);
			return out;
		};
		std::string words = (s.with.stack_for == "hdr" ? "HDR: " : "Night: ") + std::to_string(s.stacked) + " frames of " + seconds(s.frame_us) + " s merged, " + seconds(s.exposure_us) + " s in all";
		zero.ascii(0x010e, words);
	}
	zero.shorts(0x0213, 1);
	zero.longs(0x8769, 0);
	uint32_t us = uint32_t(std::clamp<int64_t>(s.exposure_us, 1, 60000000));
	uint32_t g = std::gcd(us, 1000000u);
	sub.rational(0x829a, us / g, 1000000 / g);
	sub.rational(0x829d, 20, 10);
	sub.shorts(0x8822, uint16_t(s.with.program));
	sub.shorts(0x8827, uint16_t(std::clamp(std::lround(100.0f * s.gain), 1L, 65535L)));
	sub.undefined(0x9000, { '0', '2', '3', '2' });
	sub.ascii(0x9003, when);
	sub.ascii(0x9004, when);
	sub.ascii(0x9010, zone);
	sub.ascii(0x9011, zone);
	sub.undefined(0x9101, { 1, 2, 3, 0 });
	// The compensation (a bracketing set's step among it), in thirds: SRATIONAL.
	if (s.bias_thirds != 0) {
		std::vector<uint8_t> bias = le32(uint32_t(int32_t(s.bias_thirds))), three = le32(3);
		bias.insert(bias.end(), three.begin(), three.end());
		sub.add(0x9204, 10, 1, std::move(bias));
	}
	sub.shorts(0x9207, 2);
	sub.shorts(0x9209, 0x20);
	sub.rational(0x920a, 304, 100);
	sub.undefined(0xa000, { '0', '1', '0', '0' });
	sub.shorts(0xa001, 1);
	sub.longs(0xa002, s.w);
	sub.longs(0xa003, s.h);
	sub.shorts(0xa402, 0);
	sub.shorts(0xa403, 0);
	sub.shorts(0xa405, 29);
	sub.shorts(0xa406, 0);
	sub.ascii(0xa434, LENS);
	// The look it wears (filters.rs reads it back for Photos).
	if (!s.with.look_name.empty() && !s.with.look.empty()) {
		std::vector<uint8_t> comment = { 'A', 'S', 'C', 'I', 'I', 0, 0, 0 };
		std::string words = "look: " + s.with.look_name;
		comment.insert(comment.end(), words.begin(), words.end());
		comment.push_back(0);
		uint32_t n = uint32_t(comment.size());
		sub.add(0x9286, 7, n, std::move(comment));
	}
	// The Exif directory follows the first, which points at it.
	uint32_t second = 8 + uint32_t(zero.size());
	for (Ifd::Entry &e : zero.entries)
		if (e.tag == 0x8769)
			e.data = le32(second);
	std::vector<uint8_t> out = { 'E', 'x', 'i', 'f', 0, 0 };
	std::vector<uint8_t> tiff = { 'I', 'I', 0x2a, 0, 8, 0, 0, 0 };
	zero.write(tiff, 8);
	sub.write(tiff, second);
	out.insert(out.end(), tiff.begin(), tiff.end());
	return out;
}

// ------------------------------------------------------------------ jpeg ---

// The cores a photo is made on: three, leaving one for the shell and the
// preview (photo.h makes it in bands, one a core).
const unsigned int PHOTO_THREADS = 3;

// A YUV 4:2:0 frame (packed planes) written as a JPEG at quality 95 with
// `app1` (its Exif), made on three cores, straight from its planes (no
// conversion to RGB and back, its chroma as it is). Written beside its path,
// flushed to the card and renamed into place.
bool write_jpeg(const std::string &path, const uint8_t *yuv, unsigned int w, unsigned int h, const std::vector<uint8_t> &app1, std::string &why)
{
	std::vector<uint8_t> jpeg;
	if (!photo::encode(yuv, w, h, 95, app1, PHOTO_THREADS, jpeg, why))
		return false;
	std::string part = path + ".part";
	int fd = open(part.c_str(), O_WRONLY | O_CREAT | O_TRUNC | O_CLOEXEC, 0644);
	if (fd < 0) {
		why = std::string("the photo cannot be written: ") + std::strerror(errno);
		return false;
	}
	size_t done = 0;
	while (done < jpeg.size()) {
		ssize_t n = write(fd, jpeg.data() + done, jpeg.size() - done);
		if (n < 0 && errno == EINTR)
			continue;
		if (n <= 0)
			break;
		done += size_t(n);
	}
	bool closed = done == jpeg.size() && fsync(fd) == 0;
	closed = ::close(fd) == 0 && closed;
	if (!closed || std::rename(part.c_str(), path.c_str()) != 0) {
		std::remove(part.c_str());
		why = std::string("the photo could not be kept: ") + std::strerror(errno);
		return false;
	}
	return true;
}

// ----------------------------------------------------------------- still ---

long ms_since(std::chrono::steady_clock::time_point t)
{
	return long(std::chrono::duration_cast<std::chrono::milliseconds>(std::chrono::steady_clock::now() - t).count());
}

// One of a burst's frames in a memory file of its own (packed planes, the
// right way up, cropped as the photo is), for the merge to map: -1 if there
// is no room for it.
int frame_file(const Mapped &m, unsigned int w, unsigned int h, unsigned int stride, bool turned, still::Crop c)
{
	// MFD_CLOEXEC: only the merge is given it, under a number of its own.
	int fd = int(syscall(SYS_memfd_create, "catalyst-burst", 1u));
	if (fd < 0)
		return -1;
	size_t size = size_t(c.w) * c.h * 3 / 2;
	if (ftruncate(fd, off_t(size)) != 0) {
		close(fd);
		return -1;
	}
	void *at = mmap(nullptr, size, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
	if (at == MAP_FAILED) {
		close(fd);
		return -1;
	}
	copy_frame(m, stride, w, h, c, turned, static_cast<uint8_t *>(at));
	munmap(at, size);
	return fd;
}

// A burst's or a set's next frame as the camera gives it back, or null if
// none came by `until`.
Request *next_done(std::chrono::steady_clock::time_point until)
{
	std::unique_lock<std::mutex> lock(burst_lock);
	// A stack's order ("stack end", "stack cancel") wakes it too, once.
	burst_ready.wait_until(lock, until, [] { return !burst_done.empty() || !running || stack_woken; });
	stack_woken = false;
	if (burst_done.empty())
		return nullptr;
	Request *request = burst_done.front();
	burst_done.pop_front();
	return request;
}

// An exposure, gain and white balance held fixed in a request.
ControlList held_controls(int64_t exposure_us, float gain, const float wb[2])
{
	ControlList hold;
	hold.set(controls::ExposureTimeMode, controls::ExposureTimeModeManual);
	hold.set(controls::ExposureTime, int32_t(exposure_us));
	hold.set(controls::AnalogueGainMode, controls::AnalogueGainModeManual);
	hold.set(controls::AnalogueGain, gain);
	if (wb[0] > 0 && wb[1] > 0) {
		hold.set(controls::AwbEnable, false);
		hold.set(controls::ColourGains, Span<const float, 2>({ wb[0], wb[1] }));
	}
	return hold;
}

// The first frame's exposure, analogue gain and white balance: what a
// burst holds and a set brackets around.
struct Held_as {
	int64_t exposure_us = 20000;
	float analogue = 1.0f;
	float wb[2] = { 0, 0 };
};

// The rest of a burst after its first frame (`first`, copied out): `extra`
// more frames into memory files, cropped as the photo is, the exposure and
// white balance held as the first had them. Bounded by its frames' time:
// whatever came by then is the burst.
void take_burst(Session &s, Request *first, int extra, std::vector<int> &files, still::Crop crop, const Held_as &as)
{
	{
		std::lock_guard<std::mutex> lock(burst_lock);
		burst_done.clear();
	}
	ControlList hold = held_controls(as.exposure_us, as.analogue, as.wb);
	phase = Phase::Burst;
	first->reuse(Request::ReuseBuffers);
	first->controls().merge(hold);
	if (camera->queueRequest(first) < 0)
		return;
	auto until = std::chrono::steady_clock::now() + std::chrono::milliseconds(still::deadline_ms(as.exposure_us, extra));
	int bad = 0;
	while (int(files.size()) < extra && running) {
		Request *request = next_done(until);
		if (!request) {
			std::fprintf(stderr, "still: the burst's wait ran out after %d of %d frames\n", int(files.size()), extra);
			break;
		}
		still::Status status = status_of(request, s.stream);
		if (status == still::Cancelled)
			break;
		if (status == still::Success || status == still::Startup) {
			auto m = s.planes.find(request->findBuffer(s.stream));
			if (m != s.planes.end()) {
				dma_sync(s.external_fd, true);
				int fd = frame_file(m->second, s.w, s.h, s.stride, turn_here, crop);
				dma_sync(s.external_fd, false);
				if (fd < 0)
					break;
				files.push_back(fd);
			}
		} else if (++bad >= 3) {
			break;
		}
		request->reuse(Request::ReuseBuffers);
		if (camera->queueRequest(request) < 0)
			break;
	}
}

// A bracketing set's other frames after its first (`first`, copied out):
// each asked at its own exposure and kept once a frame says it was taken
// with it (the sensor takes a new exposure a frame or two after it is
// asked; after four frames whatever comes), each a Shot of its own for the
// writer, cropped and named as the shell said.
void take_set(Session &s, Request *first, const Shot &base, still::Crop crop, const Held_as &as, std::vector<Shot> &set)
{
	{
		std::lock_guard<std::mutex> lock(burst_lock);
		burst_done.clear();
	}
	phase = Phase::Burst;
	Request *request = first;
	for (size_t i = 0; i < base.with.also.size() && running; i++) {
		const Asked::Also &more = base.with.also[i];
		ControlList want = held_controls(more.exposure_us, more.gain, as.wb);
		request->reuse(Request::ReuseBuffers);
		request->controls().merge(want);
		if (camera->queueRequest(request) < 0)
			return;
		auto until = std::chrono::steady_clock::now() + std::chrono::milliseconds(still::deadline_ms(more.exposure_us, 5));
		Request *got = nullptr;
		int seen = 0, bad = 0;
		while (running && !got) {
			Request *done = next_done(until);
			if (!done) {
				std::fprintf(stderr, "still: the set's wait ran out at its frame %zu\n", i + 2);
				return;
			}
			request = done;
			still::Status status = status_of(done, s.stream);
			if (status == still::Cancelled)
				return;
			if (status == still::Success || status == still::Startup) {
				const ControlList &said = done->metadata();
				float ag = said.get(controls::AnalogueGain).value_or(0.0f);
				bool right = still::matches(said.get(controls::ExposureTime).value_or(0), more.exposure_us) && std::fabs(ag - more.gain) <= 0.08f * more.gain + 0.05f;
				if (right || ++seen > 4) {
					got = done;
					break;
				}
			} else if (++bad >= 3) {
				return;
			}
			done->reuse(Request::ReuseBuffers);
			done->controls().merge(want);
			if (camera->queueRequest(done) < 0)
				return;
		}
		if (!got)
			return;
		auto m = s.planes.find(got->findBuffer(s.stream));
		if (m == s.planes.end())
			return;
		Shot shot;
		shot.path = more.path;
		shot.at = base.at;
		shot.asked = base.asked;
		shot.with = base.with;
		shot.with.also.clear();
		shot.with.raw.clear();
		// Its own original beside it, as filters.rs names one.
		if (!base.with.original.empty()) {
			size_t slash = base.with.original.rfind('/'), name = more.path.rfind('/');
			shot.with.original = base.with.original.substr(0, slash + 1) + more.path.substr(name == std::string::npos ? 0 : name + 1);
		}
		shot.bias_thirds = more.bias_thirds;
		const ControlList &said = got->metadata();
		shot.exposure_us = said.get(controls::ExposureTime).value_or(0);
		shot.gain = said.get(controls::AnalogueGain).value_or(1.0f) * said.get(controls::DigitalGain).value_or(1.0f);
		shot.gains[0] = as.wb[0];
		shot.gains[1] = as.wb[1];
		shot.w = crop.w;
		shot.h = crop.h;
		shot.yuv.resize(size_t(crop.w) * crop.h * 3 / 2);
		dma_sync(s.external_fd, true);
		copy_frame(m->second, s.stride, s.w, s.h, crop, turn_here, shot.yuv.data());
		dma_sync(s.external_fd, false);
		set.push_back(std::move(shot));
	}
}

// Night and HDR (1.9): the rest of a stack after its first frame (`first`,
// not yet copied out), each frame merged into the running sum as it comes
// (stack.h) while the sensor exposes the next, the exposure and white
// balance held as the first had them. One frame is copied out at a time;
// the sums are the only other memory. The first frame, made small, is the
// viewfinder's picture meanwhile. Each frame is waited for on its own
// (still.h's stack_frame_ms) and the whole within stack_ms: whatever was
// merged by then is the photo. "stack end" ends it as soon as enough is
// merged; "stack cancel" ends it with nothing (false, `cancelled`).
bool take_stack(Session &s, Request *first, Shot &shot, still::Crop crop, const Held_as &as, bool &cancelled)
{
	cancelled = false;
	const int planned = std::clamp(shot.with.stack, 2, still::STACK_MOST);
	auto m = s.planes.find(first->findBuffer(s.stream));
	if (m == s.planes.end())
		return false;
	std::vector<uint8_t> frame(size_t(crop.w) * crop.h * 3 / 2);
	dma_sync(s.external_fd, true);
	copy_frame(m->second, s.stride, s.w, s.h, crop, turn_here, frame.data());
	dma_sync(s.external_fd, false);
	{
		std::lock_guard<std::mutex> lock(burst_lock);
		burst_done.clear();
	}
	// The rest held as the first, the sensor at the next while it is merged.
	ControlList hold = held_controls(as.exposure_us, as.analogue, as.wb);
	phase = Phase::Burst;
	first->reuse(Request::ReuseBuffers);
	first->controls().merge(hold);
	bool queued = camera->queueRequest(first) >= 0;
	auto began = std::chrono::steady_clock::now();
	stack::Stack sum(crop.w, crop.h);
	sum.first(frame.data());
	int taken = 1;
	// The viewfinder: the first frame, small and toned as the photo will be
	// (only the whole picture: an aspect's crop would not fill it).
	if (colour && crop.w * 3 == crop.h * 4) {
		uint8_t table[256];
		stack::tone_table(shot.with.finish.lift, shot.with.finish.toe, table);
		std::vector<uint8_t> small;
		stack::small_copy(frame.data(), crop.w, crop.h, width, height, table, small);
		{
			std::lock_guard<std::mutex> hold_frames(lock);
			newest.swap(small);
			newest_said.clear();
			made++;
		}
		fresh.notify_one();
	}
	auto progress = [&] {
		long per = taken > 1 ? ms_since(began) / (taken - 1) : still::stack_frame_ms(as.exposure_us) / 2;
		char line[64];
		std::snprintf(line, sizeof line, "%d %d %d %ld", taken, planned, sum.frames, long(planned - taken) * per);
		tell(8, 0, line);
	};
	progress();
	const auto until_all = began + std::chrono::milliseconds(still::stack_ms(as.exposure_us, planned));
	int bad = 0;
	while (queued && taken < planned && running) {
		int order = stack_order.load();
		if (order == 2) {
			cancelled = true;
			break;
		}
		if (order == 1 && still::enough(sum.frames, planned))
			break;
		auto until = std::min(until_all, std::chrono::steady_clock::now() + std::chrono::milliseconds(still::stack_frame_ms(as.exposure_us)));
		Request *request = next_done(until);
		if (!request) {
			// Woken by an order, not a frame: looked at again.
			if (std::chrono::steady_clock::now() < until && running)
				continue;
			std::fprintf(stderr, "stack: the wait for frame %d ran out (%d of %d merged)\n", taken + 1, sum.frames, planned);
			break;
		}
		still::Status status = status_of(request, s.stream);
		if (status == still::Cancelled)
			break;
		bool copied = false;
		if (status == still::Success || status == still::Startup) {
			auto p = s.planes.find(request->findBuffer(s.stream));
			if (p != s.planes.end()) {
				dma_sync(s.external_fd, true);
				copy_frame(p->second, s.stride, s.w, s.h, crop, turn_here, frame.data());
				dma_sync(s.external_fd, false);
				copied = true;
			}
		} else if (++bad >= 3) {
			break;
		}
		// Straight back to the sensor, then merged while it exposes the next.
		request->reuse(Request::ReuseBuffers);
		if (camera->queueRequest(request) < 0)
			queued = false;
		if (!copied)
			continue;
		taken++;
		if (!sum.add(frame.data(), 3))
			std::fprintf(stderr, "stack: frame %d would not line up; left out\n", taken);
		progress();
	}
	if (cancelled || !running)
		return false;
	shot.stacked = sum.frames;
	shot.stack_planned = planned;
	shot.frame_us = as.exposure_us;
	shot.exposure_us = as.exposure_us * sum.frames;
	shot.w = crop.w;
	shot.h = crop.h;
	// Into the frame's own memory: no third full-size picture at any time.
	sum.result(frame.data());
	shot.yuv.swap(frame);
	// The filter's reach: the noise left, as merged (the finish runs as it
	// is written, the sums gone by then).
	shot.with.finish.noise = sum.residual();
	std::fprintf(stderr, "stack: %d of %d frames merged (%d taken) in %ld ms, noise %.1f, left %.1f\n", sum.frames, planned, taken, ms_since(began), double(sum.noise[8]), double(sum.residual()));
	return true;
}

// Bytes in a memory file of their own, for a helper to read: -1 if there is
// no room for them.
int bytes_file(const char *name, const uint8_t *data, size_t size)
{
	// MFD_CLOEXEC: only the helper is given it, under a number of its own.
	int fd = int(syscall(SYS_memfd_create, name, 1u));
	if (fd < 0)
		return -1;
	if (ftruncate(fd, off_t(size)) != 0) {
		close(fd);
		return -1;
	}
	void *at = mmap(nullptr, size, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
	if (at == MAP_FAILED) {
		close(fd);
		return -1;
	}
	std::memcpy(at, data, size);
	munmap(at, size);
	return fd;
}

// One try at a photo's picture at `size` into `shot`: the camera set up for
// a still (one frame of ours, in the held memory when `use_held`; the
// sensor's own frame too when `want_raw`), taken with what the shell asked
// (`shot.with`: the exposure fixed, so the first frame is the photo; else
// let settle), the frame copied out (cropped to its aspect); for detail
// mode `extra` more frames into `files`, for bracketing the set's other
// frames into `set`. Bounded: given up as soon as nothing will come, and
// after a few frames' time at most. `held_suspect` says the held frame was
// in use and no frame came: it is the first thing to doubt.
bool last_try_held = false;

// The last try's patience (deadline_ms): twice, so a sensor slow to switch
// still gives its photo when the others ran out.
static int still_patience = 1;

bool take_still_at(Size size, Shot &shot, std::string &why, bool want_raw, bool use_held, int extra, std::vector<int> *files, std::vector<Shot> *set, bool &held_suspect, bool *cancelled = nullptr)
{
	held_suspect = false;
	last_try_held = false;
	Opening how;
	how.raw = how.map_raw = want_raw;
	how.use_held = use_held;
	std::unique_ptr<Session> s = open_session(StreamRole::StillCapture, size, 1, how, why);
	shot.marks.mark("config");
	if (!s)
		return false;
	const bool held_in_use = s->external != nullptr;
	last_try_held = held_in_use;
	{
		std::lock_guard<std::mutex> hold(still_lock);
		still_judge = still::Judge();
		still_request = nullptr;
		still_failed.clear();
	}
	const Asked &a = shot.with;
	// The longest exposure this mode allows, as the pipeline says (the
	// driver's 11.77 s at the full sensor).
	int64_t longest_allowed = still::LONGEST_US;
	auto info = camera->controls().find(&controls::ExposureTime);
	if (info != camera->controls().end() && info->second.max().type() == ControlTypeInteger32 && info->second.max().get<int32_t>() > 0)
		longest_allowed = info->second.max().get<int32_t>();
	const int64_t fixed = a.exposure_us > 0 && a.gain > 0 ? std::min(a.exposure_us, longest_allowed) : 0;
	if (fixed > 0 && fixed < a.exposure_us)
		std::fprintf(stderr, "still: %lld us asked, %lld us is this mode's longest\n", (long long)a.exposure_us, (long long)fixed);
	still_fixed_us = fixed;
	const bool is_set = set && !a.also.empty() && extra <= 0;
	// Night or HDR: frames merged as they come (take_stack), never with a
	// burst or a set.
	const bool stacking = a.stack > 1 && cancelled && extra <= 0 && !is_set;
	int64_t longest = fixed;
	if (is_set)
		for (const Asked::Also &more : a.also)
			longest = std::max(longest, std::min(more.exposure_us, longest_allowed));
	ControlList controls = limits(15, std::max<int64_t>(night ? 200000 : 66666, longest + 1000));
	if (fixed > 0) {
		controls.set(controls::ExposureTimeMode, controls::ExposureTimeModeManual);
		controls.set(controls::ExposureTime, int32_t(fixed));
		controls.set(controls::AnalogueGainMode, controls::AnalogueGainModeManual);
		controls.set(controls::AnalogueGain, a.gain);
	} else {
		// Where the face-aware exposure had got to, carried into the photo.
		controls.set(controls::ExposureValue, pro_ev() + ev);
	}
	if (a.ct > 0) {
		controls.set(controls::ColourTemperature, int32_t(a.ct));
	} else if (a.gains[0] > 0 && a.gains[1] > 0) {
		controls.set(controls::AwbEnable, false);
		controls.set(controls::ColourGains, Span<const float, 2>({ a.gains[0], a.gains[1] }));
	}
	if (a.sharpness > 0)
		controls.set(controls::Sharpness, a.sharpness);
	int32_t denoise = controls::draft::NoiseReductionModeHighQuality;
	if (a.denoise == "off")
		denoise = controls::draft::NoiseReductionModeOff;
	else if (a.denoise == "fast")
		denoise = controls::draft::NoiseReductionModeFast;
	controls.set(controls::draft::NoiseReductionMode, denoise);
	if (!start(*s, std::move(controls), Phase::Still, why)) {
		shot.marks.mark("refused");
		still_fixed_us = 0;
		return false;
	}
	shot.marks.mark("start");
	// The shell's "hold still", from now for as long as its frames take at
	// most: the first frame (the sensor's start, then the frame), a burst's
	// more at every other frame, a set's each a few frames (a new exposure
	// takes a frame or two to arrive).
	const int64_t f0 = still::frame_us(fixed > 0 ? fixed : 66667);
	int64_t expect_us = 2 * f0 + std::max(0, extra) * 2 * f0;
	int frames = 1 + std::max(0, extra);
	if (stacking) {
		expect_us = int64_t(still::stack_expect_ms(fixed > 0 ? fixed : 66667, a.stack)) * 1000;
		frames = a.stack;
	}
	if (is_set)
		for (const Asked::Also &more : a.also) {
			expect_us += 5 * still::frame_us(std::min(more.exposure_us, longest_allowed));
			frames++;
		}
	char exposing[64];
	std::snprintf(exposing, sizeof exposing, "%lld %d %s", (long long)(expect_us / 1000), frames, is_set ? "bracket" : extra > 0 ? "detail" : stacking ? a.stack_for.c_str() : "still");
	tell(6, 0, exposing);
	const long wait = still::deadline_ms(fixed, still::Judge::most(fixed > 0, fixed > 1000000) + 1, still_patience);
	Request *got = nullptr;
	{
		std::unique_lock<std::mutex> hold(still_lock);
		bool ended = still_ready.wait_for(hold, std::chrono::milliseconds(wait), [] { return still_request != nullptr || !still_failed.empty() || !running; });
		got = still_request;
		if (!got)
			why = !ended ? "no frame within " + std::to_string(wait) + " ms" : !still_failed.empty() ? still_failed : "stopped";
	}
	bool ok = false;
	if (!got) {
		held_suspect = held_in_use;
		shot.marks.mark("gave-up");
		std::fprintf(stderr, "still: %s (%s frame, %ux%u) %s\n", why.c_str(), held_in_use ? "the held" : "the allocator's", size.width, size.height, shot.marks.line.c_str());
		why = "the camera gave no picture for the photo (" + why + ")";
	} else {
		shot.marks.mark("frame");
		FrameBuffer *buffer = got->findBuffer(s->stream);
		auto m = buffer ? s->planes.find(buffer) : s->planes.end();
		if (m == s->planes.end()) {
			why = "the photo's picture cannot be read";
		} else {
			const ControlList &said = got->metadata();
			Held_as as;
			as.exposure_us = said.get(controls::ExposureTime).value_or(fixed > 0 ? fixed : 20000);
			as.analogue = said.get(controls::AnalogueGain).value_or(1.0f);
			shot.exposure_us = as.exposure_us;
			shot.gain = as.analogue * said.get(controls::DigitalGain).value_or(1.0f);
			if (auto gains = said.get(controls::ColourGains)) {
				shot.gains[0] = as.wb[0] = (*gains)[0];
				shot.gains[1] = as.wb[1] = (*gains)[1];
			}
			if (auto black = said.get(controls::SensorBlackLevels))
				shot.black = (*black)[0];
			// A single photo: the sensor is done with it, the hand may move.
			if (extra <= 0 && !is_set && !stacking)
				tell(7, 0, shot.marks.line);
			const still::Crop crop = still::crop_for(s->w, s->h, a.aspect_a, a.aspect_b);
			if (stacking) {
				bool stopped = false;
				ok = take_stack(*s, got, shot, crop, as, stopped);
				*cancelled = stopped;
				shot.marks.mark("frames");
				tell(7, 0, shot.marks.line);
				if (!ok)
					why = stopped ? "cancelled" : "the stack's frames did not come";
				stop_camera();
				s->close();
				still_fixed_us = 0;
				return ok;
			}
			shot.w = crop.w;
			shot.h = crop.h;
			shot.yuv.resize(size_t(crop.w) * crop.h * 3 / 2);
			dma_sync(s->external_fd, true);
			copy_frame(m->second, s->stride, s->w, s->h, crop, turn_here, shot.yuv.data());
			dma_sync(s->external_fd, false);
			shot.marks.mark("copy");
			// The sensor's own frame, as it came, for the DNG (never cropped).
			if (want_raw) {
				FrameBuffer *raw = s->raw ? got->findBuffer(s->raw) : nullptr;
				auto rp = raw ? s->raw_planes.find(raw) : s->raw_planes.end();
				if (rp != s->raw_planes.end() && raw->metadata().status == FrameMetadata::FrameSuccess) {
					shot.raw_fd = bytes_file("catalyst-raw", rp->second, size_t(s->raw_stride) * s->raw_h);
					shot.raw_w = s->raw_w;
					shot.raw_h = s->raw_h;
					shot.raw_stride = s->raw_stride;
					shot.raw_format = s->raw_format;
				}
				shot.raw_skipped = shot.raw_fd < 0;
			}
			ok = true;
			if (extra > 0 && files)
				take_burst(*s, got, extra, *files, crop, as);
			else if (is_set)
				take_set(*s, got, shot, crop, as, *set);
			if (extra > 0 || is_set) {
				shot.marks.mark("frames");
				tell(7, 0, shot.marks.line);
			}
		}
	}
	stop_camera();
	s->close();
	still_fixed_us = 0;
	return ok;
}

// A helper started on its own at idle priority (SCHED_IDLE, nice 19, idle
// I/O), first in line for the out-of-memory killer, with nothing on this
// program's output (that is the shell's picture stream), handed `files` as
// file descriptors 3, 4, …: it runs on by itself (its own session, a
// grandchild that init takes over).
void spawn_idle(std::vector<std::string> words, const std::vector<int> &files)
{
	// Everything the child needs, made before it exists.
	std::vector<char *> argv;
	for (std::string &w : words)
		argv.push_back(w.data());
	argv.push_back(nullptr);
	pid_t child = fork();
	if (child == 0) {
		// Only system calls from here until the program is replaced.
		if (fork() != 0)
			_exit(0);
		setsid();
		int quiet = open("/dev/null", O_RDWR);
		for (int std_fd = 0; std_fd < 3; std_fd++)
			dup2(quiet, std_fd);
		// Out of the way first (a frame's number may be one of 3, 4, …).
		int high[16];
		size_t n = std::min(files.size(), size_t(16));
		for (size_t i = 0; i < n; i++)
			high[i] = fcntl(files[i], F_DUPFD, 64);
		for (size_t i = 0; i < n; i++)
			dup2(high[i], int(3 + i));
		setpriority(PRIO_PROCESS, 0, 19);
		int priority = 0;
		syscall(SYS_sched_setscheduler, 0, 5 /* SCHED_IDLE */, &priority);
		syscall(SYS_ioprio_set, 1, 0, 3 << 13);
		int oom = open("/proc/self/oom_score_adj", O_WRONLY);
		if (oom >= 0) {
			const char *v = "800";
			ssize_t ignored = write(oom, v, 3);
			(void)ignored;
			close(oom);
		}
		execv(argv[0], argv.data());
		_exit(127);
	}
	if (child > 0)
		waitpid(child, nullptr, 0);
}

// The merge for a detail photo (catalyst-shell --merge-burst, burst.rs),
// handed the frames: the photo's own first (as taken, before any look), as
// a memory file too; with the look's table and where the photo as taken is
// kept, so the merged photo wears the look as well.
void start_merge(const Shot &shot)
{
	std::vector<int> files;
	{
		Mapped m;
		m.y = shot.yuv.data();
		m.u = m.y + size_t(shot.w) * shot.h;
		m.v = m.u + size_t(shot.w / 2) * (shot.h / 2);
		int fd = frame_file(m, shot.w, shot.h, shot.w, false, still::Crop{ 0, 0, shot.w, shot.h });
		if (fd < 0) {
			for (int f : shot.burst)
				close(f);
			return;
		}
		files.push_back(fd);
	}
	files.insert(files.end(), shot.burst.begin(), shot.burst.end());
	std::vector<std::string> words = { "/usr/bin/catalyst-shell", "--merge-burst", shot.path, std::to_string(shot.w), std::to_string(shot.h) };
	if (shot.face) {
		words.push_back("face");
		for (float v : shot.face_box)
			words.push_back(std::to_string(v));
	}
	if (!shot.with.look.empty()) {
		words.push_back("look");
		words.push_back(shot.with.look);
		if (!shot.with.original.empty()) {
			words.push_back("orig");
			words.push_back(shot.with.original);
		}
	}
	words.push_back("--");
	for (size_t i = 0; i < files.size(); i++)
		words.push_back("fd:" + std::to_string(3 + i));
	spawn_idle(std::move(words), files);
	for (int f : files)
		close(f);
}

// The RAW: the sensor's frame handed to catalyst-shell --write-dng (dng.rs),
// which unpacks it and writes the DNG with the colour matrices from the
// tuning file.
void start_dng(Shot &shot)
{
	char gains[48], gain[24];
	std::snprintf(gains, sizeof gains, "gains=%.4f,%.4f", double(shot.gains[0]), double(shot.gains[1]));
	std::snprintf(gain, sizeof gain, "gain=%.3f", double(shot.gain));
	std::vector<std::string> words = {
		"/usr/bin/catalyst-shell", "--write-dng", shot.with.raw,
		"w=" + std::to_string(shot.raw_w), "h=" + std::to_string(shot.raw_h), "stride=" + std::to_string(shot.raw_stride),
		"format=" + shot.raw_format, "black=" + std::to_string(shot.black), gains,
		"exp=" + std::to_string(shot.exposure_us), gain, "time=" + std::to_string(int64_t(shot.at)),
		std::string("turned=") + (turn_here ? "1" : "0"),
	};
	if (!shot.with.look_name.empty() && !shot.with.look.empty())
		words.push_back("look=" + shot.with.look_name);
	words.push_back("--");
	words.push_back("fd:3");
	spawn_idle(std::move(words), { shot.raw_fd });
	close(shot.raw_fd);
	shot.raw_fd = -1;
}

// A photo written: the look's table over it (on three cores) and the JPEG
// made, then its RAW and merge handed on, then the photo as taken kept
// beside it.
void write_shot(Shot &shot)
{
	auto began = std::chrono::steady_clock::now();
	std::string why;
	// Night's and HDR's finish (or a single photo's lift): before the look,
	// so the photo as taken has it too.
	if (shot.with.finish.any())
		stack::finish(shot.yuv.data(), shot.w, shot.h, shot.with.finish, PHOTO_THREADS);
	long finish_ms = ms_since(began);
	photo::Lut lut = shot.with.look.empty() ? photo::Lut() : photo::read_lut(shot.with.look);
	if (lut.empty())
		shot.with.look.clear();
	std::vector<uint8_t> app1 = exif(shot);
	bool ok;
	if (lut.empty()) {
		ok = write_jpeg(shot.path, shot.yuv.data(), shot.w, shot.h, app1, why);
	} else {
		std::vector<uint8_t> looked(shot.yuv.size());
		photo::apply_lut(lut, shot.yuv.data(), looked.data(), shot.w, shot.h, PHOTO_THREADS);
		ok = write_jpeg(shot.path, looked.data(), shot.w, shot.h, app1, why);
	}
	if (!ok) {
		for (int f : shot.burst)
			close(f);
		if (shot.raw_fd >= 0)
			close(shot.raw_fd);
		tell(3, 0, why);
		return;
	}
	std::string news = "\tshot " + std::to_string(shot.shot_ms) + " write " + std::to_string(ms_since(began));
	if (!shot.burst.empty())
		news += " detail " + std::to_string(shot.burst.size() + 1);
	// A stack: "night 5 8" (merged, asked), and the finish's time.
	if (shot.stacked > 0)
		news += " " + (shot.with.stack_for.empty() ? std::string("night") : shot.with.stack_for) + " " + std::to_string(shot.stacked) + " " + std::to_string(shot.stack_planned) + " finish " + std::to_string(finish_ms);
	if (shot.raw_fd >= 0) {
		start_dng(shot);
		news += " raw";
	} else if (shot.raw_skipped) {
		news += " raw-skipped";
	}
	// The merge first, so its marker is there before the shell looks.
	if (!shot.burst.empty())
		start_merge(shot);
	tell(2, shot.w << 16 | shot.h, shot.path + news);
	// The photo as taken, before its look, for Photos' "o".
	if (!lut.empty() && !shot.with.original.empty()) {
		std::string dir = shot.with.original.substr(0, shot.with.original.rfind('/'));
		mkdir(dir.c_str(), 0755);
		shot.with.look_name.clear();
		std::string ignored;
		write_jpeg(shot.with.original, shot.yuv.data(), shot.w, shot.h, exif(shot), ignored);
	}
}

// The photos waiting to be written, one after another on a thread of their
// own behind the preview and the shell: a photo can be taken while the last
// is still being written (two at most in memory, 12 MB each).
std::mutex write_lock;
std::condition_variable write_ready, write_done;
std::deque<Shot> to_write;
bool writing_one = false;
std::thread writer_thread;

void write_photos()
{
	setpriority(PRIO_PROCESS, id_t(syscall(SYS_gettid)), 10);
	while (true) {
		Shot shot;
		{
			std::unique_lock<std::mutex> hold(write_lock);
			write_ready.wait(hold, [] { return !to_write.empty() || !running; });
			// Ending: whatever was taken is written first.
			if (to_write.empty())
				break;
			shot = std::move(to_write.front());
			to_write.pop_front();
			writing_one = true;
		}
		write_shot(shot);
		{
			std::lock_guard<std::mutex> hold(write_lock);
			writing_one = false;
		}
		write_done.notify_all();
	}
}

// Megabytes the system says it has free.
long available_mb()
{
	std::ifstream info("/proc/meminfo");
	std::string key, unit;
	long kb = 0;
	while (info >> key >> kb >> unit)
		if (key == "MemAvailable:")
			return kb / 1024;
	return -1;
}

// How many frames a detail photo can take: each is 12 MB at full size, and
// the merge needs about 30 MB besides; never more than six (most of the
// noise is gone by then), and no burst at all under three.
int burst_frames(unsigned int w, unsigned int h)
{
	long available = available_mb();
	if (available < 0)
		return 0;
	long frame = long(w) * h * 3 / 2 / (1024 * 1024) + 1;
	// Room left for everything else: 96 MB.
	long fit = (available - 96 - 30) / frame;
	return fit >= 3 ? int(std::min(fit, 6L)) : 0;
}

void take_still(const std::string &path, const std::string &words, int detail = 0);

// "detail auto|on <path>": a burst merged in the background, or (auto, out
// of real low light) a photo as usual. The shell decides "auto" itself from
// the preview's own frames (pro.rs's low_light) and asks "on" or a still;
// this is for anything that asks "auto". Real low light only: under 25 lux
// with the gain well up too, both saying so (0.9.3 took "or a gain of 8",
// which the preview reaches under about 50 lux: an evening room).
void take_detail(const std::string &how, const std::string &path, const std::string &words)
{
	bool dim = seen_lux.load() < 25.0f && seen_gain.load() >= 6.0f;
	int frames = (how == "on" || dim) ? burst_frames(3280, 2464) : 0;
	take_still(path, words, frames);
}

void take_still(const std::string &path, const std::string &words, int detail)
{
	auto asked = std::chrono::steady_clock::now();
	Shot shot;
	shot.marks.t0 = asked;
	// Room to keep it: two photos at most waiting in memory (12 MB each),
	// and the system not short. Bounded: 0.9.3 could wait twenty seconds
	// here behind a photo being written, the shell saying "hold still".
	{
		std::unique_lock<std::mutex> hold(write_lock);
		bool roomy = write_done.wait_for(hold, std::chrono::seconds(4), [] {
			size_t busy = to_write.size() + (writing_one ? 1 : 0);
			if (!running || busy == 0)
				return true;
			long mb = available_mb();
			return busy < 2 && (mb < 0 || mb > 64);
		});
		if (!roomy)
			std::fprintf(stderr, "still: no room to keep it after 4 s of waiting (photos being written); taken anyway\n");
	}
	shot.marks.mark("room");
	// A stack's order left from before (it came after its stack ended) is
	// not this photo's.
	stack_order = 0;
	{
		std::lock_guard<std::mutex> hold(burst_lock);
		stack_woken = false;
	}
	stop_camera();
	preview.reset();
	shot.marks.mark("stop");
	shot.path = path;
	shot.asked = asked;
	shot.at = std::time(nullptr);
	shot.with = asked_of(words);
	shot.bias_thirds = shot.with.bias_thirds;
	std::string why;
	{
		std::lock_guard<std::mutex> hold(face_lock);
		shot.face = face_on && std::chrono::steady_clock::now() - face_seen < std::chrono::milliseconds(800);
		std::copy(face_box, face_box + 4, shot.face_box);
	}
	// A bracketing set's frames each wait in memory to be written: as many
	// as there is room for.
	if (!shot.with.also.empty()) {
		long mb = available_mb();
		size_t fit = mb < 0 ? shot.with.also.size() : size_t(std::max(0L, (mb - 96) / 13));
		if (fit < shot.with.also.size()) {
			std::fprintf(stderr, "still: room for %zu of the set's %zu more frames\n", fit, shot.with.also.size());
			shot.with.also.resize(fit);
		}
		detail = 0;
	}
	// The full sensor, in the held frame, with the RAW if asked. Should that
	// not run: without the RAW (the pool may not stretch to it), then
	// without the held frame (given up for good if no frame came in it),
	// then once more after a breath if memory was short (pages being moved
	// out of the pool's way), and only then the binned half each way, so a
	// photo is never lost. Each try is bounded by its frames' time.
	Size full(3280, 2464);
	std::vector<Shot> set;
	// Night and HDR: merged at the full size where memory has room for the
	// sums, else binned, else a photo as usual (still.h's stack_size); never
	// with a burst, a set or a RAW.
	bool cancelled = false;
	bool *stacking = nullptr;
	if (shot.with.stack > 1) {
		stack_order = 0;
		detail = 0;
		shot.with.also.clear();
		shot.with.raw.clear();
		long mb = available_mb();
		switch (still::stack_size(mb)) {
		case still::StackSize::Full:
			stacking = &cancelled;
			break;
		case still::StackSize::Binned:
			stacking = &cancelled;
			full = Size(1640, 1232);
			std::fprintf(stderr, "stack: %ld MB free: merged binned, at 1640 x 1232\n", mb);
			break;
		case still::StackSize::None:
			shot.with.stack = 0;
			std::fprintf(stderr, "stack: %ld MB free: no room for a stack; one frame\n", mb);
			break;
		}
	}
	bool want_raw = !shot.with.raw.empty() && detail <= 1 && shot.with.also.empty();
	bool suspect = false;
	bool ok = take_still_at(full, shot, why, want_raw, !held.failed, detail - 1, &shot.burst, &set, suspect, stacking);
	// Whether a try has already run as the plainest would (no RAW, no held
	// frame): no use running it again. A stack cancelled is not tried again.
	bool plain_tried = !want_raw && !last_try_held;
	if (!ok && suspect)
		held.give_up(why);
	if (!ok && running && want_raw) {
		want_raw = false;
		shot.raw_skipped = true;
		ok = take_still_at(full, shot, why, false, !held.failed, detail - 1, &shot.burst, &set, suspect, stacking);
		plain_tried = !last_try_held;
		if (!ok && suspect)
			held.give_up(why);
	}
	if (!ok && running && !plain_tried && !cancelled)
		ok = take_still_at(full, shot, why, false, false, detail - 1, &shot.burst, &set, suspect, stacking);
	if (!ok && running && !cancelled && why.find("memory") != std::string::npos) {
		std::this_thread::sleep_for(std::chrono::milliseconds(150));
		ok = take_still_at(full, shot, why, false, false, detail - 1, &shot.burst, &set, suspect, stacking);
	}
	if (!ok && running && !cancelled) {
		std::string again;
		still_patience = 2;
		ok = take_still_at(Size(1640, 1232), shot, again, false, false, 0, nullptr, &set, suspect, stacking);
		still_patience = 1;
	}
	stack_order = 0;
	if (cancelled)
		why = "cancelled";
	// Its picture is in: the shell may take the next as soon as the preview
	// is back.
	if (ok) {
		shot.shot_ms = ms_since(asked);
		shot.marks.mark("captured");
		tell(4, shot.w << 16 | shot.h, path + "\t" + shot.marks.line);
	}
	std::string restart;
	if (running && !start_preview(restart)) {
		std::fprintf(stderr, "%s\n", restart.c_str());
		running = false;
		fresh.notify_all();
	}
	shot.marks.mark("preview");
	std::fprintf(stderr, "still: %s %s %s\n", ok ? "took" : "failed", path.c_str(), shot.marks.line.c_str());
	if (!ok) {
		tell(3, 0, why);
		return;
	}
	{
		std::lock_guard<std::mutex> hold(write_lock);
		to_write.push_back(std::move(shot));
		for (Shot &more : set)
			to_write.push_back(std::move(more));
	}
	write_ready.notify_one();
}

// ---------------------------------------------------------------- orders ---

std::mutex order_lock;
std::condition_variable order_ready;
std::deque<std::string> orders;

// Lines on standard input, handed to the main thread; the end of the input
// (the shell has gone) stops everything.
void read_orders()
{
	std::string line;
	while (running && std::getline(std::cin, line)) {
		// A stack's orders take as they come: the main thread is taking it.
		if (line == "stack end" || line == "stack cancel") {
			stack_order = line == "stack end" ? std::max(1, stack_order.load()) : 2;
			{
				std::lock_guard<std::mutex> hold(burst_lock);
				stack_woken = true;
			}
			burst_ready.notify_all();
			continue;
		}
		{
			std::lock_guard<std::mutex> hold(order_lock);
			orders.push_back(line);
		}
		order_ready.notify_one();
	}
	running = false;
	order_ready.notify_one();
	fresh.notify_all();
}

void stop(int) { running = false; }

unsigned int arg(int argc, char **argv, const char *name, unsigned int fallback)
{
	for (int i = 1; i + 1 < argc; i++)
		if (std::strcmp(argv[i], name) == 0)
			return unsigned(std::strtoul(argv[i + 1], nullptr, 10));
	return fallback;
}

const char *word(int argc, char **argv, const char *name, const char *fallback)
{
	for (int i = 1; i + 1 < argc; i++)
		if (std::strcmp(argv[i], name) == 0)
			return argv[i + 1];
	return fallback;
}

bool flag(int argc, char **argv, const char *name)
{
	for (int i = 1; i < argc; i++)
		if (std::strcmp(argv[i], name) == 0)
			return true;
	return false;
}

} // namespace

int main(int argc, char **argv)
{
	colour = flag(argc, argv, "--colour");
	width = arg(argc, argv, "--width", colour ? 320 : 640);
	height = arg(argc, argv, "--height", colour ? 240 : 480);
	fps = arg(argc, argv, "--fps", 30);
	slowest = arg(argc, argv, "--slowest", 15);
	if (fps == 0)
		fps = 30;
	if (slowest == 0 || slowest > fps)
		slowest = fps;
	if (width % 2 || height % 2) {
		std::fprintf(stderr, "the size must be even each way\n");
		return 1;
	}
	std::string way = word(argc, argv, "--orientation", "180");
	if (way == "0")
		wanted = Orientation::Rotate0;
	else if (way == "mirror")
		wanted = Orientation::Rotate0Mirror;
	else if (way == "flip")
		wanted = Orientation::Rotate180Mirror;
	else if (way != "180") {
		std::fprintf(stderr, "the orientation is 0, 180, mirror or flip, not %s\n", way.c_str());
		return 1;
	}
	std::signal(SIGINT, stop);
	std::signal(SIGTERM, stop);
	std::signal(SIGPIPE, SIG_IGN);

	// The pipeline keeping one raw frame of its own, not four, so a full-size
	// photo fits in the camera's memory (see the notes at the top). Read once,
	// as the camera manager starts.
	const char *lean = "/usr/share/catalyst-camera/vc4.yaml";
	if (!std::getenv("LIBCAMERA_RPI_CONFIG_FILE") && access(lean, R_OK) == 0)
		setenv("LIBCAMERA_RPI_CONFIG_FILE", lean, 0);

	auto manager = std::make_unique<CameraManager>();
	if (manager->start()) {
		std::fprintf(stderr, "the camera service would not start\n");
		return 1;
	}
	if (manager->cameras().empty()) {
		std::fprintf(stderr, "no cameras available\n");
		return 1;
	}
	camera = manager->cameras()[0];
	if (camera->acquire()) {
		std::fprintf(stderr, "the camera is in use\n");
		return 1;
	}
	camera->requestCompleted.connect(completed);

	std::string why;
	if (!start_preview(why)) {
		std::fprintf(stderr, "%s\n", why.c_str());
		return 1;
	}

	std::thread writer(write_frames);
	std::thread reader;
	if (colour) {
		reader = std::thread(read_orders);
		writer_thread = std::thread(write_photos);
	}

	// A photo's path, and the words after it (a tab apart).
	auto path_and_words = [](const std::string &rest) {
		size_t tab = rest.find('\t');
		return std::make_pair(rest.substr(0, tab), tab == std::string::npos ? std::string() : rest.substr(tab + 1));
	};
	// The preview started again with what was just asked (a mode change).
	auto restart = [&] {
		stop_camera();
		preview.reset();
		if (!start_preview(why)) {
			std::fprintf(stderr, "%s\n", why.c_str());
			running = false;
		}
	};

	// The main thread carries out what is asked (a photo stops and restarts
	// the camera, which cannot be done from its own thread).
	while (running) {
		std::string order;
		{
			std::unique_lock<std::mutex> hold(order_lock);
			order_ready.wait_for(hold, std::chrono::milliseconds(200), [] { return !orders.empty() || !running; });
			if (orders.empty())
				continue;
			order = orders.front();
			orders.pop_front();
		}
		if (order.rfind("still ", 0) == 0 && order.size() > 6) {
			auto [path, words] = path_and_words(order.substr(6));
			take_still(path, words);
		} else if (order.rfind("detail ", 0) == 0 && order.size() > 7) {
			std::string rest = order.substr(7);
			size_t space = rest.find(' ');
			if (space != std::string::npos) {
				auto [path, words] = path_and_words(rest.substr(space + 1));
				take_detail(rest.substr(0, space), path, words);
			}
		} else if (order.rfind("face ", 0) == 0) {
			take_face(order);
		} else if (order == "night on" || order == "night off") {
			// Into the next request with the dials' controls (frame limits
			// and all): until 1.9 the camera was restarted, the picture going
			// dark and its exposure starting over at every change.
			night = order == "night on";
			pro_dirty = true;
		} else if (order.rfind("pro ", 0) == 0) {
			ProControls p;
			char exp[24] = "", gain[24] = "", ct[24] = "";
			float stops = 0;
			long long longest = 0;
			if (std::sscanf(order.c_str(), "pro exp=%23s gain=%23s ev=%f ct=%23s max=%lld", exp, gain, &stops, ct, &longest) == 5) {
				p.exposure_us = std::strcmp(exp, "auto") ? std::strtoll(exp, nullptr, 10) : 0;
				p.gain = std::strcmp(gain, "auto") ? std::strtof(gain, nullptr) : 0;
				p.ev = std::clamp(stops, -3.0f, 3.0f);
				p.ct = std::strcmp(ct, "auto") ? std::atoi(ct) : 0;
				p.max_us = longest;
				if (const char *g = std::strstr(order.c_str(), " gains="))
					std::sscanf(g, " gains=%f,%f", &p.gains[0], &p.gains[1]);
				p.spot = std::strstr(order.c_str(), " meter=spot") != nullptr;
				{
					std::lock_guard<std::mutex> hold(pro_lock);
					pro = p;
				}
				// Into the next request, frame limits and all (the Raspberry
				// Pi pipeline takes FrameDurationLimits with any request).
				pro_dirty = true;
			}
		} else if (order.rfind("focus ", 0) == 0) {
			float x = 0.5f, y = 0.5f;
			bool on = std::sscanf(order.c_str(), "focus on %f %f", &x, &y) == 2;
			focus_x = std::clamp(x, 0.0f, 1.0f);
			focus_y = std::clamp(y, 0.0f, 1.0f);
			if (on != focus_on.load()) {
				// Into the full sensor, or back to the binned preview.
				focus_on = on;
				restart();
			} else if (on) {
				crop_dirty = true;
			}
		}
	}

	fresh.notify_all();
	writer.join();
	stop_camera();
	preview.reset();
	// Photos still being written are finished (their pictures are ours).
	write_ready.notify_all();
	if (writer_thread.joinable())
		writer_thread.join();
	camera->requestCompleted.disconnect(completed);
	camera->release();
	camera.reset();
	manager->stop();
	// The input is read until it ends: the shell closes it on leaving.
	if (reader.joinable())
		reader.detach();
	return 0;
}
