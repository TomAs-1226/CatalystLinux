// photo.h: what catalyst-camera does to a photo's frame once it has it, with
// nothing of libcamera's: the look's table laid over it, and the JPEG made
// on several cores. Kept apart so it can be built and checked on its own
// (photo-test.cpp, under qemu-aarch64: the strips decode to exactly what one
// pass makes, and the table gives what the shell's own filters.rs gives).

#pragma once


#include <algorithm>
#include <csetjmp>
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <fstream>
#include <iterator>
#include <string>
#include <thread>
#include <vector>

// After the standard headers: it uses size_t and FILE without including them.
#include <jpeglib.h>

namespace photo {

// ------------------------------------------------------------------ look ---

// A look's table (the shell's filters.rs bakes it): YUV to YUV, `n` steps
// each way, Y slowest, then U, then V, three bytes an entry.
struct Lut {
	unsigned n = 0;
	std::vector<uint8_t> table;
	bool empty() const { return n < 2; }
};

// From its file: "CLUT", its size, then the entries; empty if it is not one.
inline Lut read_lut(const std::string &path)
{
	Lut lut;
	std::ifstream in(path, std::ios::binary);
	std::vector<uint8_t> bytes((std::istreambuf_iterator<char>(in)), std::istreambuf_iterator<char>());
	if (bytes.size() < 5 || std::memcmp(bytes.data(), "CLUT", 4) != 0)
		return lut;
	unsigned n = bytes[4];
	if (n < 2 || bytes.size() != 5 + size_t(n) * n * n * 3)
		return lut;
	lut.n = n;
	lut.table.assign(bytes.begin() + 5, bytes.end());
	return lut;
}

// One colour through the table, trilinear, as filters.rs's lut_at.
inline void lut_at(const Lut &lut, uint8_t y, uint8_t u, uint8_t v, uint8_t out[3])
{
	const unsigned n = lut.n;
	auto pos = [n](uint8_t x, unsigned &i, unsigned &f) {
		unsigned p = unsigned(x) * (n - 1) * 256 / 255;
		i = std::min(p >> 8, n - 2);
		f = p - (i << 8);
	};
	unsigned y0, fy, u0, fu, v0, fv;
	pos(y, y0, fy);
	pos(u, u0, fu);
	pos(v, v0, fv);
	const uint8_t *t = lut.table.data();
	auto at = [n](unsigned a, unsigned b, unsigned c) { return ((size_t(a) * n + b) * n + c) * 3; };
	for (int ch = 0; ch < 3; ch++) {
		auto p = [&](unsigned a, unsigned b, unsigned c) { return uint32_t(t[at(a, b, c) + ch]); };
		auto lerp = [](uint32_t a, uint32_t b, uint32_t f) { return a * (256 - f) + b * f; };
		uint32_t c00 = lerp(p(y0, u0, v0), p(y0, u0, v0 + 1), fv);
		uint32_t c01 = lerp(p(y0, u0 + 1, v0), p(y0, u0 + 1, v0 + 1), fv);
		uint32_t c10 = lerp(p(y0 + 1, u0, v0), p(y0 + 1, u0, v0 + 1), fv);
		uint32_t c11 = lerp(p(y0 + 1, u0 + 1, v0), p(y0 + 1, u0 + 1, v0 + 1), fv);
		uint32_t c0 = (c00 * (256 - fu) + c01 * fu) >> 8;
		uint32_t c1 = (c10 * (256 - fu) + c11 * fu) >> 8;
		out[ch] = uint8_t(std::min<uint32_t>((c0 * (256 - fy) + c1 * fy + (1u << 15)) >> 16, 255));
	}
}

// The table over a YUV 4:2:0 frame (packed planes) from `in` into `out`
// (which may be the same), on `threads` cores: each pixel through it with
// its block's colour, the block's new colour the mean of its four.
inline void apply_lut(const Lut &lut, const uint8_t *in, uint8_t *out, unsigned w, unsigned h, unsigned threads)
{
	const unsigned cw = w / 2, ch = h / 2;
	auto rows = [&](unsigned b0, unsigned b1) {
		const uint8_t *iy = in, *iu = in + size_t(w) * h, *iv = iu + size_t(cw) * ch;
		uint8_t *oy = out, *ou = out + size_t(w) * h, *ov = ou + size_t(cw) * ch;
		for (unsigned by = b0; by < b1; by++) {
			for (unsigned bx = 0; bx < cw; bx++) {
				size_t c = size_t(by) * cw + bx;
				uint8_t u = iu[c], v = iv[c];
				unsigned su = 0, sv = 0;
				for (unsigned k = 0; k < 4; k++) {
					size_t at = size_t(2 * by + k / 2) * w + 2 * bx + k % 2;
					uint8_t o[3];
					lut_at(lut, iy[at], u, v, o);
					oy[at] = o[0];
					su += o[1];
					sv += o[2];
				}
				ou[c] = uint8_t((su + 2) / 4);
				ov[c] = uint8_t((sv + 2) / 4);
			}
		}
	};
	threads = std::max(1u, std::min(threads, ch));
	std::vector<std::thread> helpers;
	for (unsigned i = 1; i < threads; i++)
		helpers.emplace_back(rows, ch * i / threads, ch * (i + 1) / threads);
	rows(0, ch / threads);
	for (std::thread &t : helpers)
		t.join();
}

// ------------------------------------------------------------------ jpeg ---

struct JpegError {
	jpeg_error_mgr mgr;
	jmp_buf back;
	char words[JMSG_LENGTH_MAX];
};

inline void jpeg_failed(j_common_ptr c)
{
	JpegError *e = reinterpret_cast<JpegError *>(c->err);
	(*c->err->format_message)(c, e->words);
	longjmp(e->back, 1);
}

// Rows `top` to `top + rows` of a YUV 4:2:0 frame (packed planes, `w` × `h`)
// as a JPEG of their own in memory, straight from the planes (no colour
// conversion), libjpeg-turbo's accurate integer DCT with its NEON, at
// `quality`; `app1` (Exif) written after the start if given.
inline bool encode_rows(const uint8_t *yuv, unsigned w, unsigned h, unsigned top, unsigned rows, int quality, const std::vector<uint8_t> *app1, std::vector<uint8_t> &out, std::string &why)
{
	jpeg_compress_struct c;
	JpegError e;
	c.err = jpeg_std_error(&e.mgr);
	e.mgr.error_exit = jpeg_failed;
	unsigned char *mem = nullptr;
	unsigned long size = 0;
	const unsigned cw = w / 2, ch = h / 2;
	const uint8_t *py = yuv, *pu = yuv + size_t(w) * h, *pv = pu + size_t(cw) * ch;
	// libjpeg reads each raw row to a whole block (8 samples): every row is
	// copied into one of these first, its last sample repeated out to the
	// block, never reading past the picture's row.
	unsigned yw = (w + 7) & ~7u, cwp = (cw + 7) & ~7u;
	std::vector<uint8_t> keptY(size_t(yw) * 16), keptU(size_t(cwp) * 8), keptV(size_t(cwp) * 8);
	if (setjmp(e.back)) {
		jpeg_destroy_compress(&c);
		std::free(mem);
		why = std::string("the photo could not be made: ") + e.words;
		return false;
	}
	jpeg_create_compress(&c);
	jpeg_mem_dest(&c, &mem, &size);
	c.image_width = w;
	c.image_height = rows;
	c.input_components = 3;
	c.in_color_space = JCS_YCbCr;
	jpeg_set_defaults(&c);
	jpeg_set_colorspace(&c, JCS_YCbCr);
	jpeg_set_quality(&c, quality, TRUE);
	c.dct_method = JDCT_ISLOW;
	c.optimize_coding = FALSE;
	c.write_JFIF_header = FALSE;
	c.raw_data_in = TRUE;
	c.comp_info[0].h_samp_factor = 2;
	c.comp_info[0].v_samp_factor = 2;
	for (int i = 1; i < 3; i++)
		c.comp_info[i].h_samp_factor = c.comp_info[i].v_samp_factor = 1;
	jpeg_start_compress(&c, TRUE);
	if (app1)
		jpeg_write_marker(&c, JPEG_APP0 + 1, app1->data(), unsigned(app1->size()));
	JSAMPROW ys[16], us[8], vs[8];
	JSAMPARRAY planes[3] = { ys, us, vs };
	auto row = [](const uint8_t *plane, unsigned pw, unsigned ph, unsigned padded, unsigned r, uint8_t *keep) -> JSAMPROW {
		r = std::min(r, ph - 1);
		std::memcpy(keep, plane + size_t(r) * pw, pw);
		std::fill(keep + pw, keep + padded, keep[pw - 1]);
		return keep;
	};
	// Past the strip's last row (and the picture's), its last row again.
	const unsigned last = std::min(top + rows, h) - 1;
	for (unsigned at = 0; at < rows; at += 16) {
		for (unsigned i = 0; i < 16; i++)
			ys[i] = row(py, w, last + 1, yw, top + at + i, &keptY[size_t(i) * yw]);
		for (unsigned i = 0; i < 8; i++) {
			unsigned r = std::min((top + at) / 2 + i, last / 2);
			us[i] = row(pu, cw, ch, cwp, r, &keptU[size_t(i) * cwp]);
			vs[i] = row(pv, cw, ch, cwp, r, &keptV[size_t(i) * cwp]);
		}
		jpeg_write_raw_data(&c, planes, 16);
	}
	jpeg_finish_compress(&c);
	jpeg_destroy_compress(&c);
	out.assign(mem, mem + size);
	std::free(mem);
	return true;
}

// Where a JPEG's scan data starts (just after its SOS segment), and its
// frame header (SOF0); 0 if they cannot be found.
inline size_t scan_start(const std::vector<uint8_t> &j, size_t *sof = nullptr)
{
	size_t at = 2;
	while (at + 4 <= j.size() && j[at] == 0xFF) {
		uint8_t m = j[at + 1];
		size_t len = (size_t(j[at + 2]) << 8) | j[at + 3];
		if (m == 0xC0 && sof)
			*sof = at;
		if (m == 0xDA)
			return at + 2 + len;
		at += 2 + len;
	}
	return 0;
}

// A YUV 4:2:0 frame as one baseline JPEG made on `threads` cores: the frame
// cut into bands of whole MCU rows, each band encoded on its own core as a
// JPEG of its own, then joined into one scan with a restart marker between
// bands (a band starting afresh is exactly what a restart is: its DC
// predictions from zero, its last byte padded with ones), the first band's
// headers kept with the whole picture's height and the restart interval
// (one band's MCUs) added. Every decoder reads restart markers.
inline bool encode(const uint8_t *yuv, unsigned w, unsigned h, int quality, const std::vector<uint8_t> &app1, unsigned threads, std::vector<uint8_t> &jpeg, std::string &why)
{
	const unsigned mcu_rows = (h + 15) / 16, mcus_across = (w + 15) / 16;
	threads = std::max(1u, std::min(threads, mcu_rows));
	// Each band but the last the same height: the restart interval is fixed.
	unsigned band = (mcu_rows + threads - 1) / threads;
	while (size_t(band) * mcus_across > 65535 && band > 1)
		band--;
	unsigned bands = (mcu_rows + band - 1) / band;
	if (bands == 1)
		return encode_rows(yuv, w, h, 0, h, quality, &app1, jpeg, why);
	std::vector<std::vector<uint8_t>> parts(bands);
	std::vector<std::string> trouble(bands);
	std::vector<char> ok(bands, 0);
	auto work = [&](unsigned first, unsigned step) {
		for (unsigned b = first; b < bands; b += step) {
			unsigned top = b * band * 16, rows = std::min(band * 16, h - top);
			ok[b] = encode_rows(yuv, w, h, top, rows, quality, b == 0 ? &app1 : nullptr, parts[b], trouble[b]);
		}
	};
	unsigned workers = std::min(threads, bands);
	std::vector<std::thread> helpers;
	for (unsigned i = 1; i < workers; i++)
		helpers.emplace_back(work, i, workers);
	work(0, workers);
	for (std::thread &t : helpers)
		t.join();
	for (unsigned b = 0; b < bands; b++)
		if (!ok[b]) {
			why = trouble[b];
			return false;
		}
	size_t sof = 0, head = scan_start(parts[0], &sof);
	if (!head || !sof) {
		why = "the photo could not be made: no scan in its first band";
		return false;
	}
	jpeg.clear();
	size_t total = 0;
	for (auto &p : parts)
		total += p.size();
	jpeg.reserve(total + 64);
	// The headers up to the scan's, the whole height in the frame header,
	// the restart interval just before the scan.
	size_t sos = head;
	{
		size_t at = 2;
		while (at + 4 <= parts[0].size()) {
			size_t len = (size_t(parts[0][at + 2]) << 8) | parts[0][at + 3];
			if (parts[0][at + 1] == 0xDA) {
				sos = at;
				break;
			}
			at += 2 + len;
		}
	}
	jpeg.insert(jpeg.end(), parts[0].begin(), parts[0].begin() + sos);
	jpeg[sof + 5] = uint8_t(h >> 8);
	jpeg[sof + 6] = uint8_t(h);
	unsigned interval = band * mcus_across;
	const uint8_t dri[6] = { 0xFF, 0xDD, 0x00, 0x04, uint8_t(interval >> 8), uint8_t(interval) };
	jpeg.insert(jpeg.end(), dri, dri + 6);
	jpeg.insert(jpeg.end(), parts[0].begin() + sos, parts[0].begin() + head);
	for (unsigned b = 0; b < bands; b++) {
		size_t from = b == 0 ? head : scan_start(parts[b]);
		size_t to = parts[b].size() - 2; // before its EOI
		if (!from || to < from || parts[b][to] != 0xFF || parts[b][to + 1] != 0xD9) {
			why = "the photo could not be made: a band did not end as a JPEG ends";
			return false;
		}
		jpeg.insert(jpeg.end(), parts[b].begin() + from, parts[b].begin() + to);
		if (b + 1 < bands) {
			jpeg.push_back(0xFF);
			jpeg.push_back(uint8_t(0xD0 + b % 8));
		}
		std::vector<uint8_t>().swap(parts[b]);
	}
	jpeg.push_back(0xFF);
	jpeg.push_back(0xD9);
	return true;
}

} // namespace photo
