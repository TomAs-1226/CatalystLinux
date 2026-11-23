// photo-test: checks photo.h on its own, without a camera. Built with the
// image's cross compiler and run under qemu-aarch64:
//
//   aarch64-linux-g++ -std=c++17 -O2 -I../src photo-test.cpp -ljpeg -lpthread -o photo-test
//   qemu-aarch64 -L <sysroot> ./photo-test
//
// - A full-size frame made in bands on three cores decodes to exactly the
//   pixels the same frame made in one pass decodes to (and the timings).
// - The 1640 x 1232 fallback and a 320 x 240 frame too.
// - A look's table: an identity table leaves the frame within a level, a
//   grey one takes every colour to grey, and its file is read back.

#include "photo.h"

#include <chrono>
#include <cmath>
#include <cstdio>
#include <cstdlib>

static std::vector<uint8_t> frame(unsigned w, unsigned h)
{
	std::vector<uint8_t> yuv(size_t(w) * h * 3 / 2);
	uint32_t seed = 12345;
	auto noise = [&] {
		seed = seed * 1664525 + 1013904223;
		return int(seed >> 28) - 8;
	};
	for (unsigned y = 0; y < h; y++)
		for (unsigned x = 0; x < w; x++) {
			int v = int((x * 255) / w / 2 + (y * 255) / h / 2) + ((x / 37 + y / 23) % 2 ? 40 : -40) + noise();
			yuv[size_t(y) * w + x] = uint8_t(std::clamp(v, 0, 255));
		}
	uint8_t *u = yuv.data() + size_t(w) * h, *v = u + size_t(w / 2) * (h / 2);
	for (unsigned y = 0; y < h / 2; y++)
		for (unsigned x = 0; x < w / 2; x++) {
			u[size_t(y) * (w / 2) + x] = uint8_t(64 + (x * 128) / (w / 2));
			v[size_t(y) * (w / 2) + x] = uint8_t(192 - (y * 128) / (h / 2));
		}
	return yuv;
}

static bool decode(const std::vector<uint8_t> &jpeg, std::vector<uint8_t> &rgb, unsigned &w, unsigned &h)
{
	jpeg_decompress_struct d;
	photo::JpegError e;
	d.err = jpeg_std_error(&e.mgr);
	e.mgr.error_exit = photo::jpeg_failed;
	if (setjmp(e.back)) {
		std::fprintf(stderr, "decode: %s\n", e.words);
		jpeg_destroy_decompress(&d);
		return false;
	}
	jpeg_create_decompress(&d);
	jpeg_mem_src(&d, jpeg.data(), jpeg.size());
	jpeg_read_header(&d, TRUE);
	d.dct_method = JDCT_ISLOW;
	jpeg_start_decompress(&d);
	w = d.output_width;
	h = d.output_height;
	rgb.resize(size_t(w) * h * 3);
	while (d.output_scanline < d.output_height) {
		JSAMPROW row = rgb.data() + size_t(d.output_scanline) * w * 3;
		jpeg_read_scanlines(&d, &row, 1);
	}
	jpeg_finish_decompress(&d);
	jpeg_destroy_decompress(&d);
	return true;
}

static double ms(std::chrono::steady_clock::time_point t)
{
	return std::chrono::duration<double, std::milli>(std::chrono::steady_clock::now() - t).count();
}

int main()
{
	int failed = 0;
	std::vector<uint8_t> exif = { 'E', 'x', 'i', 'f', 0, 0, 'I', 'I', 42, 0, 8, 0, 0, 0, 0, 0, 0, 0, 0, 0 };
	for (auto [w, h] : { std::pair<unsigned, unsigned>{ 3280, 2464 }, { 1640, 1232 }, { 320, 240 } }) {
		std::vector<uint8_t> yuv = frame(w, h), one, three, a, b;
		std::string why;
		auto t0 = std::chrono::steady_clock::now();
		bool ok1 = photo::encode(yuv.data(), w, h, 95, exif, 1, one, why);
		double single = ms(t0);
		auto t1 = std::chrono::steady_clock::now();
		bool ok3 = photo::encode(yuv.data(), w, h, 95, exif, 3, three, why);
		double banded = ms(t1);
		unsigned w1, h1, w3, h3;
		if (!ok1 || !ok3 || !decode(one, a, w1, h1) || !decode(three, b, w3, h3)) {
			std::printf("FAIL %u x %u: %s\n", w, h, why.c_str());
			failed++;
			continue;
		}
		bool same = w1 == w && h1 == h && w3 == w && h3 == h && a == b;
		std::printf("%s %u x %u: one pass %.0f ms %zu bytes, three bands %.0f ms %zu bytes, decoded %s\n", same ? "ok  " : "FAIL", w, h, single, one.size(), banded, three.size(), same ? "identically" : "differently");
		failed += !same;
		// The Exif is in the banded photo, once.
		bool has_exif = three.size() > 12 && three[2] == 0xFF && three[3] == 0xE1;
		if (!has_exif) {
			std::printf("FAIL: no Exif first\n");
			failed++;
		}
	}
	// Tables.
	const unsigned n = 33;
	photo::Lut identity, grey;
	identity.n = grey.n = n;
	for (unsigned yi = 0; yi < n; yi++)
		for (unsigned ui = 0; ui < n; ui++)
			for (unsigned vi = 0; vi < n; vi++) {
				uint8_t e[3] = { uint8_t(std::lround(yi * 255.0 / (n - 1))), uint8_t(std::lround(ui * 255.0 / (n - 1))), uint8_t(std::lround(vi * 255.0 / (n - 1))) };
				identity.table.insert(identity.table.end(), e, e + 3);
				uint8_t g[3] = { e[0], 128, 128 };
				grey.table.insert(grey.table.end(), g, g + 3);
			}
	std::vector<uint8_t> yuv = frame(3280, 2464), out(yuv.size());
	auto t = std::chrono::steady_clock::now();
	photo::apply_lut(identity, yuv.data(), out.data(), 3280, 2464, 3);
	double lut_ms = ms(t);
	int worst = 0;
	for (size_t i = 0; i < yuv.size(); i++)
		worst = std::max(worst, std::abs(int(yuv[i]) - int(out[i])));
	std::printf("%s identity table: %d off at most, %.0f ms for 8 MP on three cores\n", worst <= 1 ? "ok  " : "FAIL", worst, lut_ms);
	failed += worst > 1;
	photo::apply_lut(grey, yuv.data(), yuv.data(), 3280, 2464, 3);
	bool all_grey = std::all_of(yuv.begin() + 3280 * 2464, yuv.end(), [](uint8_t c) { return c == 128; });
	std::printf("%s grey table: chroma all 128\n", all_grey ? "ok  " : "FAIL");
	failed += !all_grey;
	{
		FILE *f = std::fopen("look-test.lut", "wb");
		std::fwrite("CLUT", 1, 4, f);
		std::fputc(int(n), f);
		std::fwrite(grey.table.data(), 1, grey.table.size(), f);
		std::fclose(f);
		photo::Lut back = photo::read_lut("look-test.lut");
		bool same = back.n == n && back.table == grey.table;
		std::printf("%s table file read back\n", same ? "ok  " : "FAIL");
		failed += !same;
		std::remove("look-test.lut");
	}
	std::printf("%s\n", failed ? "FAILED" : "all passed");
	return failed ? 1 : 0;
}
