// catalyst-camera: the camera's picture, greyscale, on standard output, for
// the shell's AprilTag reader. Started when the reader opens and stopped
// when it closes (or when the reader stops reading).
//
//   catalyst-camera [--width 640] [--height 480] [--fps 30] [--slowest 15]
//                   [--orientation 180]
//
// Each frame is the picture's brightness (the Y plane of YUV 4:2:0), one
// byte a pixel, width × height bytes with no padding. The sensor runs in its
// full-view 2 × 2 binned mode where it has one (the IMX219's 1640 × 1232),
// scaled by the ISP, so the field of view and the lens are known; libcamera
// picks otherwise.
//
// Frames come at up to --fps a second, and no slower than --slowest: in dim
// light the exposure may take longer than a frame at --fps allows.
//
// Always the newest picture: each frame is copied out of the camera's buffer
// as it arrives and the buffer goes straight back to the camera, and the
// newest copy is what is written when the reader is ready for one. Frames the
// reader was too busy for are dropped, never queued, so what it sees is never
// late (writing from the camera's own thread held the buffers back, and the
// reader got frames a few behind).
//
// Which way up: on the CardputerZero the IMX219 sits the other way round from
// what the kernel's imx219 overlay says (it declares the Raspberry Pi camera
// board's 180°, which libcamera undoes), so without a turn the picture is
// upside down. The turn is asked of the camera pipeline (the sensor reads out
// flipped, at no cost); where it cannot, it is done here as each frame is
// copied. M5Stack's own camera app for the CardputerZero asks for the same
// turn. --orientation 0 for a camera mounted as its overlay says; "mirror"
// and "flip" (left-right, upside down) for anything else.
//
// What goes wrong is said on standard error, in one line.

#include <libcamera/libcamera.h>

#include <sys/mman.h>
#include <unistd.h>

#include <algorithm>
#include <atomic>
#include <cerrno>
#include <chrono>
#include <condition_variable>
#include <csignal>
#include <cstdio>
#include <cstdlib>
#include <cstring>
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
Stream *stream = nullptr;
unsigned int width = 640, height = 480, stride = 640;
// Each buffer's brightness plane, mapped.
std::map<FrameBuffer *, const uint8_t *> planes;

// The newest frame, copied out of the camera's buffer, and the count of
// frames copied so far; the writer takes it when it is ready for another.
std::mutex lock;
std::condition_variable fresh;
std::vector<uint8_t> newest;
uint64_t made = 0;
// Turned half round here, as each frame is copied, because the camera
// pipeline could not do it.
bool turn_here = false;

void stop(int) { running = false; }

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

// One frame's brightness into `out`, `width` × `height` with no padding
// (the ISP may pad its rows), turned half round if asked.
void copy_frame(const uint8_t *plane, uint8_t *out, bool turned)
{
	for (unsigned int y = 0; y < height; y++) {
		const uint8_t *row = plane + size_t(y) * stride;
		if (turned)
			std::reverse_copy(row, row + width, out + size_t(height - 1 - y) * width);
		else
			std::memcpy(out + size_t(y) * width, row, width);
	}
}

// On the camera's thread: copy the frame and give the buffer straight back.
void completed(Request *request)
{
	if (request->status() == Request::RequestCancelled || !running)
		return;
	FrameBuffer *buffer = request->buffers().begin()->second;
	auto plane = planes.find(buffer);
	if (plane != planes.end() && buffer->metadata().status == FrameMetadata::FrameSuccess) {
		{
			std::lock_guard<std::mutex> hold(lock);
			copy_frame(plane->second, newest.data(), turn_here);
			made++;
		}
		fresh.notify_one();
	}
	request->reuse(Request::ReuseBuffers);
	camera->queueRequest(request);
}

// The writer: the newest frame each time the reader is ready for one.
void write_frames()
{
	std::vector<uint8_t> sending(newest.size());
	uint64_t sent = 0;
	while (running) {
		{
			std::unique_lock<std::mutex> hold(lock);
			fresh.wait_for(hold, std::chrono::milliseconds(100), [&] { return made != sent || !running; });
			if (!running || made == sent)
				continue;
			std::swap(newest, sending);
			sent = made;
		}
		if (!send(sending.data(), sending.size()))
			running = false;
	}
}

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

} // namespace

int main(int argc, char **argv)
{
	width = arg(argc, argv, "--width", 640);
	height = arg(argc, argv, "--height", 480);
	unsigned int fps = arg(argc, argv, "--fps", 30);
	unsigned int slowest = arg(argc, argv, "--slowest", 15);
	if (fps == 0)
		fps = 30;
	if (slowest == 0 || slowest > fps)
		slowest = fps;
	std::string way = word(argc, argv, "--orientation", "180");
	Orientation wanted = Orientation::Rotate180;
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

	std::unique_ptr<CameraConfiguration> config = camera->generateConfiguration({ StreamRole::VideoRecording });
	if (!config || config->empty()) {
		std::fprintf(stderr, "the camera offers no picture\n");
		return 1;
	}
	config->orientation = wanted;
	StreamConfiguration &sc = config->at(0);
	sc.pixelFormat = formats::YUV420;
	sc.size = { width, height };
	sc.bufferCount = 4;
	// The full view, binned: what the lens figures are worked out for.
	SensorConfiguration sensor;
	sensor.bitDepth = 10;
	sensor.analogCrop = Rectangle(0, 0, 3280, 2464);
	sensor.outputSize = Size(1640, 1232);
	config->sensorConfig = sensor;
	if (config->validate() == CameraConfiguration::Invalid) {
		// Another sensor: let libcamera choose its mode.
		config->sensorConfig.reset();
		config->orientation = wanted;
		if (config->validate() == CameraConfiguration::Invalid) {
			std::fprintf(stderr, "the camera cannot give %ux%u\n", width, height);
			return 1;
		}
	}
	if (sc.size.width != width || sc.size.height != height || sc.pixelFormat != formats::YUV420) {
		std::fprintf(stderr, "the camera gives %s, not %ux%u YUV420\n", sc.toString().c_str(), width, height);
		return 1;
	}
	// What the pipeline could not turn, done here: half a turn is the only
	// one that keeps the frame's shape and costs no more than the copy.
	if (config->orientation != wanted) {
		if (wanted / config->orientation == Transform::Rot180) {
			turn_here = true;
		} else {
			std::fprintf(stderr, "the camera cannot be turned that way\n");
			return 1;
		}
	}
	if (camera->configure(config.get())) {
		std::fprintf(stderr, "the camera would not take its settings\n");
		return 1;
	}
	stream = sc.stream();
	stride = sc.stride;
	newest.assign(size_t(width) * height, 0);

	FrameBufferAllocator allocator(camera);
	if (allocator.allocate(stream) < 0) {
		std::fprintf(stderr, "no memory for the camera's frames\n");
		return 1;
	}
	std::vector<std::unique_ptr<Request>> requests;
	std::vector<std::pair<void *, size_t>> maps;
	for (const std::unique_ptr<FrameBuffer> &buffer : allocator.buffers(stream)) {
		const FrameBuffer::Plane &y = buffer->planes()[0];
		size_t length = size_t(y.offset) + y.length;
		void *base = mmap(nullptr, length, PROT_READ, MAP_SHARED, y.fd.get(), 0);
		if (base == MAP_FAILED) {
			std::fprintf(stderr, "the camera's frames cannot be read\n");
			return 1;
		}
		maps.emplace_back(base, length);
		planes[buffer.get()] = static_cast<const uint8_t *>(base) + y.offset;
		std::unique_ptr<Request> request = camera->createRequest();
		if (!request || request->addBuffer(stream, buffer.get())) {
			std::fprintf(stderr, "the camera would not take a request\n");
			return 1;
		}
		requests.push_back(std::move(request));
	}

	camera->requestCompleted.connect(completed);
	ControlList controls;
	// As fast as --fps, and as slow as --slowest when the light is low.
	controls.set(controls::FrameDurationLimits, { int64_t(1000000 / fps), int64_t(1000000 / slowest) });
	if (camera->start(&controls)) {
		std::fprintf(stderr, "the camera would not start\n");
		return 1;
	}
	for (std::unique_ptr<Request> &request : requests)
		camera->queueRequest(request.get());

	std::thread writer(write_frames);
	writer.join();

	camera->stop();
	camera->requestCompleted.disconnect(completed);
	for (auto &[base, length] : maps)
		munmap(base, length);
	requests.clear();
	allocator.free(stream);
	camera->release();
	camera.reset();
	manager->stop();
	return 0;
}
