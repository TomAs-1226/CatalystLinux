//! A small PDF writer: one picture a page, filling it. Colour and grey
//! pages carry their JPEG as it is (DCTDecode); black and white ones a
//! bit a pixel, run-length coded (RunLengthDecode: long runs of paper
//! cost two bytes). PDF 1.4, a cross-reference table, nothing else.

/// A page's picture.
pub enum Image {
    /// A JPEG, in colour or grey.
    Jpeg { data: Vec<u8>, w: usize, h: usize, grey: bool },
    /// Black and white: rows of bits, 1 for paper, each row whole bytes.
    Bits { rows: Vec<u8>, w: usize, h: usize },
}

/// A page: its size in points and its picture.
pub struct Page {
    pub size: (f32, f32),
    pub image: Image,
}

/// Bits from a page of paper (true) and ink, packed a row at a time.
pub fn pack_bits(paper: &[bool], w: usize, h: usize) -> Vec<u8> {
    let stride = w.div_ceil(8);
    let mut out = vec![0u8; stride * h];
    for y in 0..h {
        for x in 0..w {
            if paper[y * w + x] {
                out[y * stride + x / 8] |= 0x80 >> (x % 8);
            }
        }
    }
    out
}

/// Run-length coding as PDF reads it: a length byte, then either that
/// many plus one bytes as they are (0 to 127) or one byte repeated 257
/// less it times (129 to 255); 128 ends.
pub fn run_length(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() / 4 + 2);
    let mut i = 0;
    while i < data.len() {
        let mut run = 1;
        while i + run < data.len() && run < 128 && data[i + run] == data[i] {
            run += 1;
        }
        if run >= 2 {
            out.push((257 - run) as u8);
            out.push(data[i]);
            i += run;
            continue;
        }
        // As they are, up to where a run of two or more begins.
        let start = i;
        while i < data.len() && i - start < 128 && !(i + 1 < data.len() && data[i + 1] == data[i]) {
            i += 1;
        }
        if i == start {
            i += 1;
        }
        out.push((i - start - 1) as u8);
        out.extend_from_slice(&data[start..i]);
    }
    out.push(128);
    out
}

/// What [`run_length`] makes, read back.
#[cfg(test)]
pub fn run_length_back(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < data.len() {
        let n = data[i] as usize;
        i += 1;
        match n {
            128 => break,
            0..=127 => {
                out.extend_from_slice(&data[i..i + n + 1]);
                i += n + 1;
            }
            _ => {
                out.extend(std::iter::repeat_n(data[i], 257 - n));
                i += 1;
            }
        }
    }
    out
}

/// The document: the catalogue (1), the page tree (2), then for each page
/// its page, its contents and its picture.
pub fn write(pages: &[Page], title: &str) -> Vec<u8> {
    let mut out: Vec<u8> = b"%PDF-1.4\n%\xe2\xe3\xcf\xd3\n".to_vec();
    let mut offsets: Vec<usize> = Vec::new();
    let n = pages.len();
    // Objects: 1 catalogue, 2 pages, then 3 + 3i page, 4 + 3i contents,
    // 5 + 3i image; the document's details last.
    let info = 3 + 3 * n;
    let object = |out: &mut Vec<u8>, offsets: &mut Vec<usize>, body: &[u8]| {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", offsets.len()).as_bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    };
    object(&mut out, &mut offsets, b"<< /Type /Catalog /Pages 2 0 R >>");
    let kids: Vec<String> = (0..n).map(|i| format!("{} 0 R", 3 + 3 * i)).collect();
    object(&mut out, &mut offsets, format!("<< /Type /Pages /Kids [{}] /Count {n} >>", kids.join(" ")).as_bytes());
    for (i, page) in pages.iter().enumerate() {
        let (pw, ph) = page.size;
        let (contents, image) = (4 + 3 * i, 5 + 3 * i);
        object(
            &mut out,
            &mut offsets,
            format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {pw:.2} {ph:.2}] /Resources << /XObject << /Im0 {image} 0 R >> >> /Contents {contents} 0 R >>").as_bytes(),
        );
        let draw = format!("q {pw:.2} 0 0 {ph:.2} 0 0 cm /Im0 Do Q");
        object(&mut out, &mut offsets, format!("<< /Length {} >>\nstream\n{draw}\nendstream", draw.len()).as_bytes());
        let (head, data) = match &page.image {
            Image::Jpeg { data, w, h, grey } => {
                let space = if *grey { "/DeviceGray" } else { "/DeviceRGB" };
                (format!("<< /Type /XObject /Subtype /Image /Width {w} /Height {h} /ColorSpace {space} /BitsPerComponent 8 /Filter /DCTDecode /Length {} >>", data.len()), data.clone())
            }
            Image::Bits { rows, w, h } => {
                let coded = run_length(rows);
                (format!("<< /Type /XObject /Subtype /Image /Width {w} /Height {h} /ColorSpace /DeviceGray /BitsPerComponent 1 /Filter /RunLengthDecode /Length {} >>", coded.len()), coded)
            }
        };
        let mut body = head.into_bytes();
        body.extend_from_slice(b"\nstream\n");
        body.extend_from_slice(&data);
        body.extend_from_slice(b"\nendstream");
        object(&mut out, &mut offsets, &body);
    }
    let title: String = title.chars().filter(|c| c.is_ascii_graphic() || *c == ' ').filter(|c| !matches!(c, '(' | ')' | '\\')).collect();
    object(&mut out, &mut offsets, format!("<< /Title ({title}) /Producer (Catalyst Linux) /Creator (Camera: scan doc) >>").as_bytes());
    debug_assert_eq!(offsets.len(), info);
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len() + 1).as_bytes());
    for at in &offsets {
        out.extend_from_slice(format!("{at:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R /Info {info} 0 R >>\nstartxref\n{xref}\n%%EOF\n", offsets.len() + 1).as_bytes());
    out
}
