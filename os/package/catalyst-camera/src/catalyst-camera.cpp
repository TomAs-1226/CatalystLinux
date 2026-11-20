// catalyst-camera: the camera's picture, greyscale, on standard output, for
// the shell's AprilTag reader. Started when the reader opens and stopped
// when it closes (or when the reader stops reading).
//
//   catalyst-camera [--width 640] [--height 480] [--fps 15]
//
// Each frame is the picture's brightness (the Y plane of YUV 4:2:0), one
// byte a pixel, width × height bytes with no padding. The sensor runs in its
// full-view 2 × 2 binned mode where it has one (the IMX219's 1640 × 1232),
// scaled by the ISP, so the field of view and the lens are known; libcamera
// picks otherwise.
//
// What goes wrong is said on standard error, in one line.

#include <libcamera/libcamera.h>

#include <sys/mman.h>
#include <unistd.h>

#include <atomic>
#include <cerrno>
#include <csignal>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <map>
#include <memory>
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
std::vector<uint8_t> row_out;

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

void completed(Request *request)
{
	if (request->status() == Request::RequestCancelled || !running)
		return;
	FrameBuffer *buffer = request->buffers().begin()->second;
	auto plane = planes.find(buffer);
	if (plane != planes.end() && buffer->metadata().status == FrameMetadata::FrameSuccess) {
		// Row by row: the ISP may pad its rows; the reader wants none.
		bool ok = true;
		if (stride == width) {
			ok = send(plane->second, size_t(width) * height);
		} else {
			for (unsigned int y = 0; y < height && ok; y++)
				ok = send(plane->second + size_t(y) * stride, width);
		}
		if (!ok) {
			running = false;
			return;
		}
	}
	request->reuse(Request::ReuseBuffers);
	camera->queueRequest(request);
}

unsigned int arg(int argc, char **argv, const char *name, unsigned int fallback)
{
	for (int i = 1; i + 1 < argc; i++)
		if (std::strcmp(argv[i], name) == 0)
			return unsigned(std::strtoul(argv[i + 1], nullptr, 10));
	return fallback;
}

} // namespace

int main(int argc, char **argv)
{
	width = arg(argc, argv, "--width", 640);
	height = arg(argc, argv, "--height", 480);
	unsigned int fps = arg(argc, argv, "--fps", 15);
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
		if (config->validate() == CameraConfiguration::Invalid) {
			std::fprintf(stderr, "the camera cannot give %ux%u\n", width, height);
			return 1;
		}
	}
	if (sc.size.width != width || sc.size.height != height || sc.pixelFormat != formats::YUV420) {
		std::fprintf(stderr, "the camera gives %s, not %ux%u YUV420\n", sc.toString().c_str(), width, height);
		return 1;
	}
	if (camera->configure(config.get())) {
		std::fprintf(stderr, "the camera would not take its settings\n");
		return 1;
	}
	stream = sc.stream();
	stride = sc.stride;

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
	int64_t frame = 1000000 / int64_t(fps ? fps : 15);
	controls.set(controls::FrameDurationLimits, { frame, frame });
	if (camera->start(&controls)) {
		std::fprintf(stderr, "the camera would not start\n");
		return 1;
	}
	for (std::unique_ptr<Request> &request : requests)
		camera->queueRequest(request.get());

	while (running)
		std::this_thread::sleep_for(std::chrono::milliseconds(100));

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
