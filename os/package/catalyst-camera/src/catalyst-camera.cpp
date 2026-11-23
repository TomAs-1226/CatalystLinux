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
//           written); the number is width << 16 | height
//
// and lines on standard input ask for things:
//
//   still <path>      a photo at the sensor's full resolution, 3280 × 2464,
//                     as a JPEG at <path> (written beside it and renamed
//                     into place, so a half-written photo is never seen).
//                     The preview stops while the picture is taken and runs
//                     again while it is written.
//   detail auto|on <path>
//                     detail mode: the photo as above, then up to five more
//                     full-size frames with its exposure and white balance
//                     held, handed (in memory files) to the merge, which
//                     runs on by itself at idle priority and replaces the
//                     photo when done (catalyst-shell --merge-burst, see
//                     burst.rs). "auto" takes the burst only in dim light;
//                     the frames are as many as memory leaves room for.
//   face x0 y0 x1 y1  where a face is, 0 to 1 across this camera's picture
//   face none         (from the smart camera on the Grove port): the
//                     exposure leans toward it
//   night on|off      longer exposures allowed (down to 5 frames a second,
//                     a fifth of a second for a photo) when the light is low
//
// The sensor runs in its full-view 2 × 2 binned mode where it has one (the
// IMX219's 1640 × 1232), scaled by the ISP, so the field of view and the
// lens are known; libcamera picks otherwise.
//
// Memory: every camera buffer comes from the kernel's contiguous pool (CMA,
// 64 MB, shared with the GPU, which keeps 16 MB of it for its binner while
// the shell draws with it). libcamera's Raspberry Pi pipeline keeps four raw
// frames of its own by default: at the full 3280 × 2464 that is 39 MB, and
// with the ISP's output frame and ours (12 MB each) 63 MB, which never fit,
// so until 1.3 every photo fell back to the binned 1640 × 1232. Now the
// pipeline is told to keep one raw frame of its own (vc4.yaml beside this
// program, through LIBCAMERA_RPI_CONFIG_FILE): a photo needs 35 MB. The
// preview gives the sensor raw frames of ours in each request instead, so it
// still never waits for one. The photo's frame is copied out of the camera's
// memory at once and the camera handed back to the preview; the JPEG is
// written from the copy, on a thread of its own at a lower priority.
//
// Photos: the ISP's own 4:2:0 written straight as JPEG at quality 95 (no
// colour conversion, nothing resampled; 4:4:4 would need the ISP to give
// 24-bit frames, two of 24 MB, which the pool cannot hold beside the raw
// frame), its high-quality colour denoise, the exposure settled before the
// frame is kept, and Exif: the camera, the lens, the exposure, the time.
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

#include <fcntl.h>
#include <jpeglib.h>
#include <sys/mman.h>
#include <sys/wait.h>
#include <sys/resource.h>
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
		if (allocator && raw)
			allocator->free(raw);
		allocator.reset();
		config.reset();
		stream = nullptr;
		raw = nullptr;
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
enum class Phase { Preview, Still, Burst, Stopped };

// Detail mode's burst: each frame handed to the main thread as it comes.
std::mutex burst_lock;
std::condition_variable burst_ready;
std::deque<Request *> burst_done;

// The light as the preview last saw it (lux, and the gain it took), for
// detail mode's "auto": on in dim light.
std::atomic<float> seen_lux{ 400.0f };
std::atomic<float> seen_gain{ 1.0f };
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
// Frames let go by before the one kept: at least one (the first can carry
// the preview's controls), until the exposure says it has settled on the new
// mode, at most this many (the pipeline has already dropped the frames its
// algorithms asked for at the start).
const int SETTLE_MIN = 1;
const int SETTLE_MAX = 8;

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

// On the camera's thread: copy the frame and give the buffer straight back
// (a photo's frame is kept until it is written).
void completed(Request *request)
{
	Session *session = current.load();
	if (request->status() == Request::RequestCancelled || !running || !session)
		return;
	FrameBuffer *buffer = request->findBuffer(session->stream);
	if (!buffer)
		return;
	bool good = buffer->metadata().status == FrameMetadata::FrameSuccess;
	if (phase == Phase::Still) {
		std::lock_guard<std::mutex> hold(still_lock);
		// Kept once the exposure has settled on the new mode (or, in light
		// that keeps it searching, after a few frames whatever it says).
		auto ae = request->metadata().get(controls::AeState);
		bool settled = ae && *ae == controls::AeStateConverged;
		if (good && !still_request && (++still_frames > SETTLE_MAX || (settled && still_frames > SETTLE_MIN))) {
			still_request = request;
			still_ready.notify_one();
			return;
		}
		request->reuse(Request::ReuseBuffers);
		camera->queueRequest(request);
		return;
	}
	if (phase == Phase::Burst) {
		{
			std::lock_guard<std::mutex> hold(burst_lock);
			burst_done.push_back(request);
		}
		burst_ready.notify_one();
		return;
	}
	if (phase != Phase::Preview)
		return;
	{
		const ControlList &said = request->metadata();
		if (auto lux = said.get(controls::Lux))
			seen_lux = *lux;
		seen_gain = said.get(controls::AnalogueGain).value_or(1.0f) * said.get(controls::DigitalGain).value_or(1.0f);
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
			} else {
				newest.resize(size_t(w) * h);
				copy_plane(m.y, s, w, h, newest.data(), turn_here);
			}
			made++;
		}
		fresh.notify_one();
		bool lean = lean_on_face(plane->second, session->w, session->h, session->stride);
		request->reuse(Request::ReuseBuffers);
		if (lean)
			request->controls().set(controls::ExposureValue, ev);
		camera->queueRequest(request);
		return;
	}
	request->reuse(Request::ReuseBuffers);
	camera->queueRequest(request);
}

// The camera set up for `role` at `size`, in YUV 4:2:0, turned as wanted;
// `binned` asks for the sensor's full-view binned mode, `raw` for the
// sensor's frames in each request too (the preview: see Session). Empty with
// `why` said when it cannot be.
std::unique_ptr<Session> open_session(StreamRole role, Size size, unsigned int buffers, bool binned, bool raw, std::string &why)
{
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
	if (binned) {
		// The full view, binned: what the lens figures are worked out for.
		SensorConfiguration sensor;
		sensor.bitDepth = 10;
		sensor.analogCrop = Rectangle(0, 0, 3280, 2464);
		sensor.outputSize = Size(1640, 1232);
		s->config->sensorConfig = sensor;
	}
	if (s->config->validate() == CameraConfiguration::Invalid) {
		// A raw stream it will not match: the same without one.
		if (raw)
			return open_session(role, size, buffers, binned, false, why);
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
	if (s->allocator->allocate(s->stream) < 0) {
		why = "no memory for the camera's frames";
		return nullptr;
	}
	if (s->raw && s->allocator->allocate(s->raw) < 0) {
		why = "no memory for the camera's frames";
		return nullptr;
	}
	const auto &raws = s->raw ? s->allocator->buffers(s->raw) : s->allocator->buffers(s->stream);
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
	preview = open_session(StreamRole::VideoRecording, Size(width, height), 4, true, true, why);
	if (!preview)
		return false;
	// The exposure and white balance free again (a burst holds them).
	ControlList controls = limits(fps, preview_longest());
	controls.set(controls::ExposureTimeMode, controls::ExposureTimeModeAuto);
	controls.set(controls::AnalogueGainMode, controls::AnalogueGainModeAuto);
	controls.set(controls::AwbEnable, true);
	controls.set(controls::ExposureValue, ev);
	return start(*preview, std::move(controls), Phase::Preview, why);
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

// ------------------------------------------------------------------ shot ---

// A photo's picture, copied out of the camera's memory (packed planes, Y
// then U then V, the right way up), and what the camera said of it.
struct Shot {
	std::string path;
	unsigned int w = 0, h = 0;
	std::vector<uint8_t> yuv;
	int64_t exposure_us = 0;
	float gain = 1.0f;
	std::time_t at = 0;
	std::chrono::steady_clock::time_point asked;
	long shot_ms = 0;
	// Detail mode: the burst's other frames (memory files), and the face
	// the smart camera saw, if any, for the merge.
	std::vector<int> burst;
	bool face = false;
	float face_box[4] = { 0, 0, 0, 0 };
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
	zero.shorts(0x0213, 1);
	zero.longs(0x8769, 0);
	uint32_t us = uint32_t(std::clamp<int64_t>(s.exposure_us, 1, 60000000));
	uint32_t g = std::gcd(us, 1000000u);
	sub.rational(0x829a, us / g, 1000000 / g);
	sub.rational(0x829d, 20, 10);
	sub.shorts(0x8822, 2);
	sub.shorts(0x8827, uint16_t(std::clamp(std::lround(100.0f * s.gain), 1L, 65535L)));
	sub.undefined(0x9000, { '0', '2', '3', '2' });
	sub.ascii(0x9003, when);
	sub.ascii(0x9004, when);
	sub.ascii(0x9010, zone);
	sub.ascii(0x9011, zone);
	sub.undefined(0x9101, { 1, 2, 3, 0 });
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

// A photo's YUV 4:2:0, as the ISP made it, written as a JPEG straight from
// its planes (no conversion to RGB and back, its chroma as it is), with its
// Exif. Written beside its path and renamed into place.
bool write_jpeg(const Shot &shot, std::string &why)
{
	const std::string &path = shot.path;
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
	// one of these first, its last sample repeated out to the block, never
	// reading past the picture's row.
	const unsigned int w = shot.w, h = shot.h, stride = w;
	const bool turned = false;
	Mapped m;
	m.y = shot.yuv.data();
	m.u = m.y + size_t(w) * h;
	m.v = m.u + size_t(w / 2) * (h / 2);
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
	jpeg_set_quality(&c, 95, TRUE);
	c.dct_method = JDCT_ISLOW;
	// Exif first, as cameras write it (no JFIF header before it).
	c.write_JFIF_header = FALSE;
	c.raw_data_in = TRUE;
	c.comp_info[0].h_samp_factor = 2;
	c.comp_info[0].v_samp_factor = 2;
	c.comp_info[1].h_samp_factor = 1;
	c.comp_info[1].v_samp_factor = 1;
	c.comp_info[2].h_samp_factor = 1;
	c.comp_info[2].v_samp_factor = 1;
	jpeg_start_compress(&c, TRUE);
	std::vector<uint8_t> app1 = exif(shot);
	jpeg_write_marker(&c, JPEG_APP0 + 1, app1.data(), unsigned(app1.size()));
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

long ms_since(std::chrono::steady_clock::time_point t)
{
	return long(std::chrono::duration_cast<std::chrono::milliseconds>(std::chrono::steady_clock::now() - t).count());
}

// One of a burst's frames in a memory file of its own (packed planes, the
// right way up), for the merge to map: -1 if there is no room for it.
int frame_file(const Mapped &m, unsigned int w, unsigned int h, unsigned int stride, bool turned)
{
	// MFD_CLOEXEC: only the merge is given it, under a number of its own.
	int fd = int(syscall(SYS_memfd_create, "catalyst-burst", 1u));
	if (fd < 0)
		return -1;
	size_t size = size_t(w) * h * 3 / 2;
	if (ftruncate(fd, off_t(size)) != 0) {
		close(fd);
		return -1;
	}
	void *at = mmap(nullptr, size, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
	if (at == MAP_FAILED) {
		close(fd);
		return -1;
	}
	uint8_t *out = static_cast<uint8_t *>(at);
	copy_plane(m.y, stride, w, h, out, turned);
	copy_plane(m.u, stride / 2, w / 2, h / 2, out + size_t(w) * h, turned);
	copy_plane(m.v, stride / 2, w / 2, h / 2, out + size_t(w) * h * 5 / 4, turned);
	munmap(at, size);
	return fd;
}

// The rest of a burst after its first frame (`first`, settled): the
// exposure and white balance held where the first had them, `extra` more
// frames into memory files.
void take_burst(Session &s, Request *first, int extra, std::vector<int> &files)
{
	const ControlList &said = first->metadata();
	ControlList hold;
	hold.set(controls::ExposureTimeMode, controls::ExposureTimeModeManual);
	hold.set(controls::ExposureTime, said.get(controls::ExposureTime).value_or(20000));
	hold.set(controls::AnalogueGainMode, controls::AnalogueGainModeManual);
	hold.set(controls::AnalogueGain, said.get(controls::AnalogueGain).value_or(1.0f));
	if (auto gains = said.get(controls::ColourGains)) {
		hold.set(controls::AwbEnable, false);
		hold.set(controls::ColourGains, *gains);
	}
	{
		std::lock_guard<std::mutex> lock(burst_lock);
		burst_done.clear();
	}
	phase = Phase::Burst;
	first->reuse(Request::ReuseBuffers);
	first->controls().merge(hold);
	camera->queueRequest(first);
	auto until = std::chrono::steady_clock::now() + std::chrono::seconds(5);
	while (int(files.size()) < extra && running && std::chrono::steady_clock::now() < until) {
		Request *request = nullptr;
		{
			std::unique_lock<std::mutex> lock(burst_lock);
			burst_ready.wait_until(lock, until, [] { return !burst_done.empty() || !running; });
			if (burst_done.empty())
				break;
			request = burst_done.front();
			burst_done.pop_front();
		}
		FrameBuffer *buffer = request->findBuffer(s.stream);
		auto m = buffer ? s.planes.find(buffer) : s.planes.end();
		if (m != s.planes.end() && buffer->metadata().status == FrameMetadata::FrameSuccess) {
			int fd = frame_file(m->second, s.w, s.h, s.stride, turn_here);
			if (fd < 0)
				break;
			files.push_back(fd);
		}
		request->reuse(Request::ReuseBuffers);
		camera->queueRequest(request);
	}
}

// A photo's picture at `size` into `shot`: the camera set up for a still
// (one frame of ours, the pipeline keeping one raw frame), its colour
// denoise at its best, the exposure let settle, the frame copied out; and
// for detail mode `extra` more frames into `files`.
bool take_still_at(Size size, Shot &shot, std::string &why, int extra = 0, std::vector<int> *files = nullptr)
{
	std::unique_ptr<Session> s = open_session(StreamRole::StillCapture, size, 1, false, false, why);
	if (!s)
		return false;
	{
		std::lock_guard<std::mutex> hold(still_lock);
		still_frames = 0;
		still_request = nullptr;
	}
	ControlList controls = limits(15, night ? 200000 : 66666);
	controls.set(controls::draft::NoiseReductionMode, controls::draft::NoiseReductionModeHighQuality);
	// Where the face-aware exposure had got to, carried into the photo.
	controls.set(controls::ExposureValue, ev);
	if (!start(*s, std::move(controls), Phase::Still, why))
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
		FrameBuffer *buffer = got->findBuffer(s->stream);
		auto m = buffer ? s->planes.find(buffer) : s->planes.end();
		if (m == s->planes.end()) {
			why = "the photo's picture cannot be read";
		} else {
			unsigned int w = s->w, h = s->h, st = s->stride;
			shot.w = w;
			shot.h = h;
			shot.yuv.resize(size_t(w) * h * 3 / 2);
			uint8_t *out = shot.yuv.data();
			copy_plane(m->second.y, st, w, h, out, turn_here);
			copy_plane(m->second.u, st / 2, w / 2, h / 2, out + size_t(w) * h, turn_here);
			copy_plane(m->second.v, st / 2, w / 2, h / 2, out + size_t(w) * h * 5 / 4, turn_here);
			const ControlList &said = got->metadata();
			shot.exposure_us = said.get(controls::ExposureTime).value_or(0);
			shot.gain = said.get(controls::AnalogueGain).value_or(1.0f) * said.get(controls::DigitalGain).value_or(1.0f);
			ok = true;
			if (extra > 0 && files)
				take_burst(*s, got, extra, *files);
		}
	}
	stop_camera();
	s->close();
	return ok;
}

// The photo being written (one at a time), while the preview runs again.
std::thread writing;

// The merge for a detail photo (catalyst-shell --merge-burst, burst.rs),
// handed the frames as file descriptors 3, 4, …: the photo's own first, as
// a memory file too. It runs on by itself (its own session, a grandchild
// that init takes over), at idle priority (SCHED_IDLE, nice 19, idle I/O)
// and first in line for the out-of-memory killer, with nothing on this
// program's output (that is the shell's picture stream).
void start_merge(const Shot &shot)
{
	std::vector<int> files;
	{
		Mapped m;
		m.y = shot.yuv.data();
		m.u = m.y + size_t(shot.w) * shot.h;
		m.v = m.u + size_t(shot.w / 2) * (shot.h / 2);
		int fd = frame_file(m, shot.w, shot.h, shot.w, false);
		if (fd < 0) {
			for (int f : shot.burst)
				close(f);
			return;
		}
		files.push_back(fd);
	}
	files.insert(files.end(), shot.burst.begin(), shot.burst.end());
	// Everything the child needs, made before it exists.
	std::vector<std::string> words = { "/usr/bin/catalyst-shell", "--merge-burst", shot.path, std::to_string(shot.w), std::to_string(shot.h) };
	if (shot.face) {
		words.push_back("face");
		for (float v : shot.face_box)
			words.push_back(std::to_string(v));
	}
	words.push_back("--");
	for (size_t i = 0; i < files.size(); i++)
		words.push_back("fd:" + std::to_string(3 + i));
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
	for (int f : files)
		close(f);
}

void write_shot(Shot shot)
{
	// Behind the preview and the shell: the photo can take its time.
	setpriority(PRIO_PROCESS, id_t(syscall(SYS_gettid)), 10);
	auto began = std::chrono::steady_clock::now();
	std::string why;
	if (write_jpeg(shot, why)) {
		std::string detail = shot.burst.empty() ? "" : " detail " + std::to_string(shot.burst.size() + 1);
		// The merge first, so its marker is there before the shell looks.
		if (!shot.burst.empty())
			start_merge(shot);
		tell(2, shot.w << 16 | shot.h, shot.path + "\tshot " + std::to_string(shot.shot_ms) + " write " + std::to_string(ms_since(began)) + detail);
	} else {
		for (int f : shot.burst)
			close(f);
		tell(3, 0, why);
	}
}

// How many frames a detail photo can take: each is 12 MB at full size, and
// the merge needs about 30 MB besides; never more than six (most of the
// noise is gone by then), and no burst at all under three.
int burst_frames(unsigned int w, unsigned int h)
{
	std::ifstream info("/proc/meminfo");
	std::string key;
	long kb = 0, available = -1;
	std::string unit;
	while (info >> key >> kb >> unit)
		if (key == "MemAvailable:") {
			available = kb / 1024;
			break;
		}
	if (available < 0)
		return 0;
	long frame = long(w) * h * 3 / 2 / (1024 * 1024) + 1;
	// Room left for everything else: 96 MB.
	long fit = (available - 96 - 30) / frame;
	return fit >= 3 ? int(std::min(fit, 6L)) : 0;
}

void take_still(const std::string &path, int detail = 0);

// "detail auto|on <path>": a burst merged in the background, or (auto, in
// good light) a photo as usual.
void take_detail(const std::string &how, const std::string &path)
{
	bool dim = seen_lux.load() < 60.0f || seen_gain.load() >= 3.0f;
	int frames = (how == "on" || dim) ? burst_frames(3280, 2464) : 0;
	take_still(path, frames);
}

void take_still(const std::string &path, int detail)
{
	auto asked = std::chrono::steady_clock::now();
	if (writing.joinable())
		writing.join();
	stop_camera();
	preview.reset();
	Shot shot;
	shot.path = path;
	shot.asked = asked;
	shot.at = std::time(nullptr);
	std::string why;
	// The full sensor. Should the pool be short a moment (pages being moved
	// out of its way), once more after a breath; only if that fails too the
	// binned half each way, so a photo is never lost.
	{
		std::lock_guard<std::mutex> hold(face_lock);
		shot.face = face_on && std::chrono::steady_clock::now() - face_seen < std::chrono::milliseconds(800);
		std::copy(face_box, face_box + 4, shot.face_box);
	}
	bool ok = take_still_at(Size(3280, 2464), shot, why, detail - 1, &shot.burst);
	if (!ok && running) {
		std::this_thread::sleep_for(std::chrono::milliseconds(150));
		ok = take_still_at(Size(3280, 2464), shot, why, detail - 1, &shot.burst);
	}
	if (!ok && running) {
		std::string again;
		ok = take_still_at(Size(1640, 1232), shot, again);
	}
	std::string restart;
	if (running && !start_preview(restart)) {
		std::fprintf(stderr, "%s\n", restart.c_str());
		running = false;
		fresh.notify_all();
	}
	if (!ok) {
		tell(3, 0, why);
		return;
	}
	shot.shot_ms = ms_since(asked);
	tell(4, shot.w << 16 | shot.h, path);
	writing = std::thread(write_shot, std::move(shot));
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
		} else if (order.rfind("detail ", 0) == 0 && order.size() > 7) {
			std::string rest = order.substr(7);
			size_t space = rest.find(' ');
			if (space != std::string::npos)
				take_detail(rest.substr(0, space), rest.substr(space + 1));
		} else if (order.rfind("face ", 0) == 0) {
			take_face(order);
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
	// A photo still being written is finished (its picture is already ours).
	if (writing.joinable())
		writing.join();
	camera->requestCompleted.disconnect(completed);
	camera->release();
	camera.reset();
	manager->stop();
	// The input is read until it ends: the shell closes it on leaving.
	if (reader.joinable())
		reader.detach();
	return 0;
}
