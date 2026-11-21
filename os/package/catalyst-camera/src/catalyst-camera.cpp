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
//   kind 2  a photo was saved: its path; the number is width << 16 | height
//   kind 3  a photo could not be taken: why, in a line
//
// and lines on standard input ask for things:
//
//   still <path>      a photo at the sensor's full resolution (3280 × 2464,
//                     or its binned 1640 × 1232 where the camera's memory
//                     will not stretch to it), as a JPEG at <path> (written
//                     beside it and renamed into place, so a half-written
//                     photo is never seen). The preview stops while it is
//                     taken and carries on afterwards.
//   night on|off      longer exposures allowed (down to 5 frames a second,
//                     a fifth of a second for a photo) when the light is low
//
// The sensor runs in its full-view 2 × 2 binned mode where it has one (the
// IMX219's 1640 × 1232), scaled by the ISP, so the field of view and the
// lens are known; libcamera picks otherwise.
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

#include <jpeglib.h>
#include <sys/mman.h>
#include <unistd.h>

#include <algorithm>
#include <atomic>
#include <cerrno>
#include <chrono>
#include <condition_variable>
#include <csetjmp>
#include <csignal>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <deque>
#include <iostream>
#include <map>
#include <memory>
#include <mutex>
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
				sent = made;
				frame = true;
			}
		}
		for (const Message &m : say) {
			if (!send_header(m.kind, uint32_t(m.text.size()), m.number) || !send(reinterpret_cast<const uint8_t *>(m.text.data()), m.text.size()))
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
struct Session {
	std::unique_ptr<CameraConfiguration> config;
	Stream *stream = nullptr;
	unsigned int w = 0, h = 0, stride = 0;
	std::unique_ptr<FrameBufferAllocator> allocator;
	std::map<FrameBuffer *, Mapped> planes;
	std::vector<std::pair<void *, size_t>> maps;
	std::vector<std::unique_ptr<Request>> requests;

	~Session() { close(); }

	void close()
	{
		requests.clear();
		for (auto &[base, length] : maps)
			munmap(base, length);
		maps.clear();
		planes.clear();
		if (allocator && stream)
			allocator->free(stream);
		allocator.reset();
		config.reset();
		stream = nullptr;
	}
};

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

// What the camera thread does with a finished request.
enum class Phase { Preview, Still, Stopped };
std::atomic<Phase> phase{Phase::Stopped};
// The session running, read on the camera's thread as the main thread
// changes it (a photo stops the preview, takes a still, starts it again).
std::atomic<Session *> current{nullptr};

// A photo being taken: frames let through for the exposure to settle, and
// the request holding the one kept.
std::mutex still_lock;
std::condition_variable still_ready;
int still_frames = 0;
Request *still_request = nullptr;
// Frames let go by before the one kept: the exposure and white balance
// settle on the new mode.
const int SETTLE = 3;

// On the camera's thread: copy the frame and give the buffer straight back
// (a photo's frame is kept until it is written).
void completed(Request *request)
{
	Session *session = current.load();
	if (request->status() == Request::RequestCancelled || !running || !session)
		return;
	FrameBuffer *buffer = request->buffers().begin()->second;
	bool good = buffer->metadata().status == FrameMetadata::FrameSuccess;
	if (phase == Phase::Still) {
		std::lock_guard<std::mutex> hold(still_lock);
		if (good && ++still_frames > SETTLE && !still_request) {
			still_request = request;
			still_ready.notify_one();
			return;
		}
		request->reuse(Request::ReuseBuffers);
		camera->queueRequest(request);
		return;
	}
	if (phase != Phase::Preview)
		return;
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
			} else {
				newest.resize(size_t(w) * h);
				copy_plane(m.y, s, w, h, newest.data(), turn_here);
			}
			made++;
		}
		fresh.notify_one();
	}
	request->reuse(Request::ReuseBuffers);
	camera->queueRequest(request);
}

// The camera set up for `role` at `size`, in YUV 4:2:0, turned as wanted;
// `binned` asks for the sensor's full-view binned mode. Empty with `why`
// said when it cannot be.
std::unique_ptr<Session> open_session(StreamRole role, Size size, unsigned int buffers, bool binned, std::string &why)
{
	auto s = std::make_unique<Session>();
	s->config = camera->generateConfiguration({ role });
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
	if (binned) {
		// The full view, binned: what the lens figures are worked out for.
		SensorConfiguration sensor;
		sensor.bitDepth = 10;
		sensor.analogCrop = Rectangle(0, 0, 3280, 2464);
		sensor.outputSize = Size(1640, 1232);
		s->config->sensorConfig = sensor;
	}
	if (s->config->validate() == CameraConfiguration::Invalid) {
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
	s->w = sc.size.width;
	s->h = sc.size.height;
	s->stride = sc.stride;
	s->allocator = std::make_unique<FrameBufferAllocator>(camera);
	if (s->allocator->allocate(s->stream) < 0) {
		why = "no memory for the camera's frames";
		return nullptr;
	}
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

int64_t preview_longest()
{
	return night ? 200000 : int64_t(1000000 / slowest);
}

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
	for (std::unique_ptr<Request> &request : s.requests)
		camera->queueRequest(request.get());
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
	preview = open_session(StreamRole::VideoRecording, Size(width, height), 4, true, why);
	if (!preview)
		return false;
	return start(*preview, limits(fps, preview_longest()), Phase::Preview, why);
}

// ------------------------------------------------------------------ jpeg ---

struct JpegError {
	jpeg_error_mgr mgr;
	jmp_buf back;
	char words[JMSG_LENGTH_MAX];
};

void jpeg_failed(j_common_ptr c)
{
	JpegError *e = reinterpret_cast<JpegError *>(c->err);
	(*c->err->format_message)(c, e->words);
	longjmp(e->back, 1);
}

// A YUV 4:2:0 picture, as the camera's buffer holds it, written as a JPEG
// (straight from its planes: no conversion to RGB and back), turned half
// round if asked. Written beside `path` and renamed into place.
bool write_jpeg(const std::string &path, const Mapped &m, unsigned int w, unsigned int h, unsigned int stride, bool turned, std::string &why)
{
	std::string part = path + ".part";
	FILE *f = std::fopen(part.c_str(), "wb");
	if (!f) {
		why = std::string("the photo cannot be written: ") + std::strerror(errno);
		return false;
	}
	jpeg_compress_struct c;
	JpegError e;
	c.err = jpeg_std_error(&e.mgr);
	e.mgr.error_exit = jpeg_failed;
	// libjpeg reads each raw row to a whole block (8 samples): 1640 × 1232's
	// chroma rows are 820 wide and read as 824. So every row is copied into
	// one of these first (turned, if asked), its last sample repeated out to
	// the block, never reading past the camera's row or this buffer.
	unsigned int cw = w / 2, ch = h / 2, cstride = stride / 2;
	unsigned int yw = (w + 7) & ~7u, cwp = (cw + 7) & ~7u;
	std::vector<uint8_t> keptY(size_t(yw) * 16), keptU(size_t(cwp) * 8), keptV(size_t(cwp) * 8);
	if (setjmp(e.back)) {
		jpeg_destroy_compress(&c);
		std::fclose(f);
		std::remove(part.c_str());
		why = std::string("the photo could not be made: ") + e.words;
		return false;
	}
	jpeg_create_compress(&c);
	jpeg_stdio_dest(&c, f);
	c.image_width = w;
	c.image_height = h;
	c.input_components = 3;
	c.in_color_space = JCS_YCbCr;
	jpeg_set_defaults(&c);
	jpeg_set_colorspace(&c, JCS_YCbCr);
	jpeg_set_quality(&c, 92, TRUE);
	c.raw_data_in = TRUE;
	c.comp_info[0].h_samp_factor = 2;
	c.comp_info[0].v_samp_factor = 2;
	c.comp_info[1].h_samp_factor = 1;
	c.comp_info[1].v_samp_factor = 1;
	c.comp_info[2].h_samp_factor = 1;
	c.comp_info[2].v_samp_factor = 1;
	jpeg_start_compress(&c, TRUE);
	JSAMPROW ys[16], us[8], vs[8];
	JSAMPARRAY rows[3] = { ys, us, vs };
	// A row of a plane, the picture's way up, `pw` wide and padded to `padded`;
	// past the bottom, the last row again (whole groups of 16 rows are
	// written).
	auto row = [&](const uint8_t *plane, unsigned int ps, unsigned int pw, unsigned int ph, unsigned int padded, unsigned int r, uint8_t *keep) -> JSAMPROW {
		r = std::min(r, ph - 1);
		const uint8_t *from = plane + size_t(turned ? ph - 1 - r : r) * ps;
		if (turned)
			std::reverse_copy(from, from + pw, keep);
		else
			std::memcpy(keep, from, pw);
		std::fill(keep + pw, keep + padded, keep[pw - 1]);
		return keep;
	};
	for (unsigned int top = 0; top < h; top += 16) {
		for (unsigned int i = 0; i < 16; i++)
			ys[i] = row(m.y, stride, w, h, yw, top + i, &keptY[size_t(i) * yw]);
		for (unsigned int i = 0; i < 8; i++) {
			us[i] = row(m.u, cstride, cw, ch, cwp, top / 2 + i, &keptU[size_t(i) * cwp]);
			vs[i] = row(m.v, cstride, cw, ch, cwp, top / 2 + i, &keptV[size_t(i) * cwp]);
		}
		jpeg_write_raw_data(&c, rows, 16);
	}
	jpeg_finish_compress(&c);
	jpeg_destroy_compress(&c);
	bool closed = std::fflush(f) == 0 && fsync(fileno(f)) == 0;
	closed = std::fclose(f) == 0 && closed;
	if (!closed || std::rename(part.c_str(), path.c_str()) != 0) {
		std::remove(part.c_str());
		why = std::string("the photo could not be kept: ") + std::strerror(errno);
		return false;
	}
	return true;
}

// ----------------------------------------------------------------- still ---

// A photo at `size`: the preview stopped, the camera set up for a still,
// a few frames let go by for the exposure to settle, the next written.
bool take_still_at(const std::string &path, Size size, std::string &why)
{
	std::unique_ptr<Session> s = open_session(StreamRole::StillCapture, size, 1, false, why);
	if (!s)
		return false;
	{
		std::lock_guard<std::mutex> hold(still_lock);
		still_frames = 0;
		still_request = nullptr;
	}
	if (!start(*s, limits(15, night ? 200000 : 66666), Phase::Still, why))
		return false;
	Request *got = nullptr;
	{
		std::unique_lock<std::mutex> hold(still_lock);
		still_ready.wait_for(hold, std::chrono::seconds(6), [] { return still_request != nullptr || !running; });
		got = still_request;
	}
	bool ok = false;
	if (!got) {
		why = "the camera gave no picture for the photo";
	} else {
		FrameBuffer *buffer = got->buffers().begin()->second;
		auto m = s->planes.find(buffer);
		if (m == s->planes.end())
			why = "the photo's picture cannot be read";
		else
			ok = write_jpeg(path, m->second, s->w, s->h, s->stride, turn_here, why);
	}
	stop_camera();
	s->close();
	return ok;
}

void take_still(const std::string &path)
{
	stop_camera();
	preview.reset();
	std::string why;
	// The full sensor if the camera's memory stretches to it; else its
	// binned half each way, which every IMX219 gives from less.
	Size taken(3280, 2464);
	bool ok = take_still_at(path, taken, why);
	if (!ok && running) {
		taken = Size(1640, 1232);
		std::string again;
		ok = take_still_at(path, taken, again);
		if (!ok)
			why = again;
	}
	if (ok)
		tell(2, taken.width << 16 | taken.height, path);
	else
		tell(3, 0, why);
	std::string restart;
	if (running && !start_preview(restart)) {
		std::fprintf(stderr, "%s\n", restart.c_str());
		running = false;
		fresh.notify_all();
	}
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
	if (colour)
		reader = std::thread(read_orders);

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
			take_still(order.substr(6));
		} else if (order == "night on" || order == "night off") {
			night = order == "night on";
			// The new limits take with the camera's next start.
			stop_camera();
			preview.reset();
			if (!start_preview(why)) {
				std::fprintf(stderr, "%s\n", why.c_str());
				running = false;
			}
		}
	}

	fresh.notify_all();
	writer.join();
	stop_camera();
	preview.reset();
	camera->requestCompleted.disconnect(completed);
	camera->release();
	camera.reset();
	manager->stop();
	// The input is read until it ends: the shell closes it on leaving.
	if (reader.joinable())
		reader.detach();
	return 0;
}
