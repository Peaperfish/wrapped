//! Bounded EXIF reading for photos: JPEG, HEIC/HEIF and TIFF-based raws
//! (ARW, DNG, ...). Only metadata is read, never pixel data, and every read
//! goes through [`Capped`], so no file costs more than [`HARD_CAP`] bytes.
//!
//! Container handling is ours, the TIFF/IFD parsing is `kamadak-exif`'s:
//! * JPEG: walk the segment headers (seeking over each) to the `APP1 Exif`
//!   segment (at most 64 KB) and stop at the first scan.
//! * HEIF: `kamadak-exif`'s ISOBMFF reader, which seeks over `mdat` and reads
//!   only the `meta` box and the Exif item's extent, through the cap.
//! * TIFF raws: `kamadak-exif` would read the whole file (60 MB for an ARW),
//!   so we read the first [`HEAD_WINDOW`] (64 KB) and parse that with
//!   `continue_on_error`. If IFD0, the Exif IFD or the values of the tags we
//!   keep lie further, the window is extended just far enough, never past
//!   [`HARD_CAP`]. (A Sony ARW has its Exif IFD about 5 KB in.)

use std::io::{self, Read, Seek, SeekFrom};

use chrono::NaiveDateTime;
use exif::{Context, In, Tag, Value};

/// First read for TIFF-based files.
pub const HEAD_WINDOW: u64 = 64 * 1024;
/// No file is read beyond this many bytes in total.
pub const HARD_CAP: u64 = 2 * 1024 * 1024;

const TAG_IMAGE_NUMBER: Tag = Tag(Context::Exif, 0x9211);

/// The metadata kept for one photo.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PhotoMeta {
    /// DateTimeOriginal as written (camera local time).
    pub taken: Option<NaiveDateTime>,
    /// SubSecTimeOriginal digits as written.
    pub subsec: Option<String>,
    /// OffsetTimeOriginal, minutes east of UTC.
    pub offset_min: Option<i32>,
    pub make: Option<String>,
    pub model: Option<String>,
    pub serial: Option<String>,
    pub image_number: Option<u32>,
    pub lens: Option<String>,
    pub focal_mm: Option<f64>,
    pub f_number: Option<f64>,
    pub exposure_s: Option<f64>,
    pub iso: Option<u32>,
}

#[derive(Debug)]
pub enum MetaError {
    Io(io::Error),
    /// Not a JPEG, HEIF or TIFF file.
    Unsupported,
    /// No readable EXIF block (or it lies beyond the read cap).
    NoExif,
}

impl From<io::Error> for MetaError {
    fn from(e: io::Error) -> Self {
        MetaError::Io(e)
    }
}

/// A reader that counts bytes read and refuses to read more than `cap`.
pub struct Capped<R> {
    inner: R,
    cap: u64,
    pub read: u64,
}

impl<R> Capped<R> {
    pub fn new(inner: R, cap: u64) -> Self {
        Self {
            inner,
            cap,
            read: 0,
        }
    }
}

impl<R: Read> Read for Capped<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let left = self.cap.saturating_sub(self.read);
        if left == 0 && !buf.is_empty() {
            return Err(io::Error::other("read cap reached"));
        }
        let n = buf.len().min(left as usize);
        let got = self.inner.read(&mut buf[..n])?;
        self.read += got as u64;
        Ok(got)
    }
}

impl<R: Seek> Seek for Capped<R> {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        self.inner.seek(pos)
    }
}

/// Read the metadata of one file of `len` bytes. Never reads more than
/// [`HARD_CAP`] bytes; the count is in `r.read` afterwards. A truncated file
/// or one whose Exif lies beyond the cap is `NoExif`; `Io` is left for real
/// read failures (worth retrying later).
pub fn read_meta<R: Read + Seek>(r: &mut Capped<R>, len: u64) -> Result<PhotoMeta, MetaError> {
    match read_meta_inner(r, len) {
        Err(MetaError::Io(e)) if e.kind() == io::ErrorKind::UnexpectedEof || r.read >= r.cap => {
            Err(MetaError::NoExif)
        }
        other => other,
    }
}

fn read_meta_inner<R: Read + Seek>(r: &mut Capped<R>, len: u64) -> Result<PhotoMeta, MetaError> {
    let mut head = [0u8; 16];
    let n = read_up_to(r, &mut head)?;
    let head = &head[..n];
    let tiff = if head.starts_with(&[0xFF, 0xD8]) {
        jpeg_exif(r)?
    } else if head.len() >= 12 && &head[4..8] == b"ftyp" {
        r.seek(SeekFrom::Start(0))?;
        let mut br = io::BufReader::with_capacity(4 * 1024, &mut *r);
        let exif = exif::Reader::new()
            .continue_on_error(true)
            .read_from_container(&mut br)
            .or_else(|e| e.distill_partial_result(|_| {}))
            .map_err(|e| match e {
                exif::Error::Io(e) => MetaError::Io(e),
                _ => MetaError::NoExif,
            })?;
        return Ok(extract(&exif));
    } else if head.starts_with(b"II*\0") || head.starts_with(b"MM\0*") {
        return tiff_meta(r, head, len);
    } else {
        return Err(MetaError::Unsupported);
    };
    parse_tiff(tiff).map(|e| extract(&e))
}

fn read_up_to<R: Read>(r: &mut R, buf: &mut [u8]) -> io::Result<usize> {
    let mut n = 0;
    while n < buf.len() {
        match r.read(&mut buf[n..])? {
            0 => break,
            k => n += k,
        }
    }
    Ok(n)
}

fn parse_tiff(buf: Vec<u8>) -> Result<exif::Exif, MetaError> {
    exif::Reader::new()
        .continue_on_error(true)
        .read_raw(buf)
        .or_else(|e| e.distill_partial_result(|_| {}))
        .map_err(|_| MetaError::NoExif)
}

/// JPEG: the TIFF block of the first `APP1 Exif` segment (after SOI).
fn jpeg_exif<R: Read + Seek>(r: &mut Capped<R>) -> Result<Vec<u8>, MetaError> {
    let mut pos = 2u64;
    for _ in 0..64 {
        r.seek(SeekFrom::Start(pos))?;
        let mut m = [0u8; 4];
        if read_up_to(r, &mut m)? < 4 || m[0] != 0xFF {
            return Err(MetaError::NoExif);
        }
        let marker = m[1];
        // Start of scan / end of image: no Exif before the pixel data.
        if marker == 0xDA || marker == 0xD9 {
            return Err(MetaError::NoExif);
        }
        let seg_len = u64::from(u16::from_be_bytes([m[2], m[3]]));
        if seg_len < 2 {
            return Err(MetaError::NoExif);
        }
        if marker == 0xE1 && seg_len >= 8 {
            let mut seg = vec![0u8; (seg_len - 2) as usize];
            r.read_exact(&mut seg)?;
            if seg.starts_with(b"Exif\0\0") {
                seg.drain(..6);
                return Ok(seg);
            }
        }
        pos += 2 + seg_len;
    }
    Err(MetaError::NoExif)
}

/// TIFF-based raws: windowed read, extended once if the Exif IFD is further.
/// `head` holds the bytes already read from the start of the file. Reads a
/// [`HEAD_WINDOW`] first, then extends it (contiguously, never past
/// [`HARD_CAP`]) only as far as IFD0, the Exif IFD and the values of the tags
/// we keep actually reach.
fn tiff_meta<R: Read + Seek>(
    r: &mut Capped<R>,
    head: &[u8],
    len: u64,
) -> Result<PhotoMeta, MetaError> {
    let want = len.min(HEAD_WINDOW).min(HARD_CAP).max(head.len() as u64);
    let mut buf = vec![0u8; want as usize];
    buf[..head.len()].copy_from_slice(head);
    r.seek(SeekFrom::Start(head.len() as u64))?;
    let got = read_up_to(r, &mut buf[head.len()..])?;
    buf.truncate(head.len() + got);
    // Each round can reveal one more level (IFD0 → Exif IFD → its values).
    for _ in 0..4 {
        let need = needed_end(&buf).min(len);
        if need <= buf.len() as u64 || need > HARD_CAP {
            break;
        }
        let old = buf.len();
        buf.resize(need as usize, 0);
        let got = read_up_to(r, &mut buf[old..])?;
        buf.truncate(old + got);
        if got == 0 {
            break;
        }
    }
    parse_tiff(buf).map(|e| extract(&e))
}

/// Tags whose values we keep: IFD0 (Make, Model) and the Exif IFD.
const IFD0_TAGS: &[u16] = &[0x010F, 0x0110, 0x8769];
const EXIF_TAGS: &[u16] = &[
    0x829A, 0x829D, 0x8827, 0x9003, 0x9011, 0x9211, 0x9291, 0x920A, 0xA431, 0xA434,
];

/// Minimal TIFF view for planning reads (works on a truncated buffer).
struct Tiff<'a> {
    b: &'a [u8],
    le: bool,
}

impl Tiff<'_> {
    fn u16_at(&self, o: u64) -> Option<u16> {
        let o = usize::try_from(o).ok()?;
        let s: [u8; 2] = self.b.get(o..o.checked_add(2)?)?.try_into().ok()?;
        Some(if self.le {
            u16::from_le_bytes(s)
        } else {
            u16::from_be_bytes(s)
        })
    }
    fn u32_at(&self, o: u64) -> Option<u32> {
        let o = usize::try_from(o).ok()?;
        let s: [u8; 4] = self.b.get(o..o.checked_add(4)?)?.try_into().ok()?;
        Some(if self.le {
            u32::from_le_bytes(s)
        } else {
            u32::from_be_bytes(s)
        })
    }
    /// End of an IFD's table and of the out-of-line values of `tags`, or
    /// the first byte still missing to know that.
    fn ifd_end(&self, ifd: u64, tags: &[u16], exif_ptr: &mut Option<u64>) -> u64 {
        let Some(n) = self.u16_at(ifd) else {
            return ifd + 2;
        };
        let table_end = ifd + 2 + 12 * u64::from(n) + 4;
        if table_end > self.b.len() as u64 {
            return table_end;
        }
        let mut end = table_end;
        for i in 0..u64::from(n) {
            let e = ifd + 2 + 12 * i;
            let (Some(tag), Some(typ), Some(count), Some(val)) = (
                self.u16_at(e),
                self.u16_at(e + 2),
                self.u32_at(e + 4),
                self.u32_at(e + 8),
            ) else {
                continue;
            };
            if !tags.contains(&tag) {
                continue;
            }
            if tag == 0x8769 {
                *exif_ptr = Some(u64::from(val));
                continue;
            }
            let unit: u64 = match typ {
                1 | 2 | 6 | 7 => 1,
                3 | 8 => 2,
                4 | 9 | 11 => 4,
                5 | 10 | 12 => 8,
                _ => continue,
            };
            let size = unit * u64::from(count);
            if size > 4 && size <= 4096 {
                end = end.max(u64::from(val) + size);
            }
        }
        end
    }
}

/// How many bytes from the start of the file are needed to parse IFD0, the
/// Exif IFD and the values we keep (0 if not a TIFF header).
fn needed_end(b: &[u8]) -> u64 {
    let le = match b.get(0..2) {
        Some(b"II") => true,
        Some(b"MM") => false,
        _ => return 0,
    };
    let t = Tiff { b, le };
    let Some(ifd0) = t.u32_at(4).map(u64::from) else {
        return 8;
    };
    let mut exif = None;
    let mut end = t.ifd_end(ifd0, IFD0_TAGS, &mut exif);
    if end > b.len() as u64 {
        return end;
    }
    if let Some(ifd) = exif {
        end = end.max(t.ifd_end(ifd, EXIF_TAGS, &mut None));
    }
    end
}

fn ascii(exif: &exif::Exif, tag: Tag) -> Option<String> {
    let f = exif
        .get_field(tag, In::PRIMARY)
        .or_else(|| exif.fields().find(|f| f.tag == tag))?;
    let Value::Ascii(v) = &f.value else {
        return None;
    };
    let s = String::from_utf8_lossy(v.first()?);
    let s = s.trim_matches(|c: char| c == '\0' || c.is_whitespace());
    (!s.is_empty()).then(|| s.to_string())
}

fn rational(exif: &exif::Exif, tag: Tag) -> Option<f64> {
    let f = exif.get_field(tag, In::PRIMARY)?;
    let v = match &f.value {
        Value::Rational(v) => v.first()?.to_f64(),
        Value::SRational(v) => v.first()?.to_f64(),
        _ => return None,
    };
    (v.is_finite() && v > 0.0).then_some(v)
}

fn uint(exif: &exif::Exif, tag: Tag) -> Option<u32> {
    exif.get_field(tag, In::PRIMARY)?.value.get_uint(0)
}

/// "+08:00" / "-05:30" / "Z" → minutes east of UTC.
fn parse_offset(s: &str) -> Option<i32> {
    if s == "Z" {
        return Some(0);
    }
    let sign = match s.as_bytes().first()? {
        b'+' => 1,
        b'-' => -1,
        _ => return None,
    };
    let (h, m) = s[1..].split_once(':')?;
    let (h, m): (i32, i32) = (h.parse().ok()?, m.parse().ok()?);
    ((0..=14).contains(&h) && (0..60).contains(&m)).then_some(sign * (h * 60 + m))
}

fn extract(exif: &exif::Exif) -> PhotoMeta {
    let taken = ascii(exif, Tag::DateTimeOriginal)
        .and_then(|s| NaiveDateTime::parse_from_str(&s, "%Y:%m:%d %H:%M:%S").ok());
    let subsec =
        ascii(exif, Tag::SubSecTimeOriginal).filter(|s| s.chars().all(|c| c.is_ascii_digit()));
    PhotoMeta {
        taken,
        subsec,
        offset_min: ascii(exif, Tag::OffsetTimeOriginal).and_then(|s| parse_offset(&s)),
        make: ascii(exif, Tag::Make),
        model: ascii(exif, Tag::Model),
        serial: ascii(exif, Tag::BodySerialNumber),
        image_number: uint(exif, TAG_IMAGE_NUMBER),
        lens: ascii(exif, Tag::LensModel),
        focal_mm: rational(exif, Tag::FocalLength),
        f_number: rational(exif, Tag::FNumber),
        exposure_s: rational(exif, Tag::ExposureTime),
        iso: uint(exif, Tag::PhotographicSensitivity),
    }
}

impl PhotoMeta {
    /// Identity of the shot: the same exposure read from the RAW and the JPEG
    /// (or from a moved / renamed copy) gives the same id.
    pub fn shot_id(&self) -> Option<String> {
        let taken = self.taken?;
        let key = format!(
            "{}\0{}\0{}\0{}\0{}\0{}",
            self.make.as_deref().unwrap_or(""),
            self.model.as_deref().unwrap_or(""),
            self.serial.as_deref().unwrap_or(""),
            taken.format("%Y-%m-%dT%H:%M:%S"),
            self.subsec.as_deref().unwrap_or(""),
            self.image_number.map(|n| n.to_string()).unwrap_or_default(),
        );
        Some(super::hash128_hex(key.as_bytes()))
    }

    /// Milliseconds from SubSecTimeOriginal ("5" = 500 ms, "123" = 123 ms).
    pub fn subsec_ms(&self) -> i64 {
        let Some(s) = &self.subsec else { return 0 };
        let mut digits: String = s.chars().take(3).collect();
        while digits.len() < 3 {
            digits.push('0');
        }
        digits.parse().unwrap_or(0)
    }
}

/// Builders for EXIF fixtures (tests here and in `tests/`): a TIFF block with
/// IFD0 (Make, Model, Exif pointer) and an Exif IFD, and containers around it.
#[doc(hidden)]
pub mod fixture {
    /// Fields to write; `None` = tag absent.
    #[derive(Debug, Clone, Default)]
    pub struct Shot {
        pub make: Option<&'static str>,
        pub model: Option<&'static str>,
        pub taken: Option<&'static str>,
        pub subsec: Option<&'static str>,
        pub offset: Option<&'static str>,
        pub serial: Option<&'static str>,
        pub lens: Option<&'static str>,
        pub focal: Option<(u32, u32)>,
        pub f_number: Option<(u32, u32)>,
        pub exposure: Option<(u32, u32)>,
        pub iso: Option<u16>,
    }

    enum V {
        Ascii(String),
        Short(u16),
        Long(u32),
        Rational(u32, u32),
    }

    /// Little-endian TIFF. `exif_ifd_at`: put the Exif IFD at this absolute
    /// offset (padding with zeros), to test windowed reads of large raws.
    pub fn tiff(s: &Shot, exif_ifd_at: Option<u32>) -> Vec<u8> {
        let mut ifd0: Vec<(u16, V)> = Vec::new();
        if let Some(m) = s.make {
            ifd0.push((0x010F, V::Ascii(m.into())));
        }
        if let Some(m) = s.model {
            ifd0.push((0x0110, V::Ascii(m.into())));
        }
        ifd0.push((0x8769, V::Long(0))); // patched below
        let mut ex: Vec<(u16, V)> = Vec::new();
        if let Some(v) = s.exposure {
            ex.push((0x829A, V::Rational(v.0, v.1)));
        }
        if let Some(v) = s.f_number {
            ex.push((0x829D, V::Rational(v.0, v.1)));
        }
        if let Some(v) = s.iso {
            ex.push((0x8827, V::Short(v)));
        }
        if let Some(v) = s.taken {
            ex.push((0x9003, V::Ascii(v.into())));
        }
        if let Some(v) = s.offset {
            ex.push((0x9011, V::Ascii(v.into())));
        }
        if let Some(v) = s.subsec {
            ex.push((0x9291, V::Ascii(v.into())));
        }
        if let Some(v) = s.focal {
            ex.push((0x920A, V::Rational(v.0, v.1)));
        }
        if let Some(v) = s.serial {
            ex.push((0xA431, V::Ascii(v.into())));
        }
        if let Some(v) = s.lens {
            ex.push((0xA434, V::Ascii(v.into())));
        }
        let mut out = b"II*\0\x08\0\0\0".to_vec();
        let ifd0_at = 8u32;
        let ifd0_len = 2 + 12 * ifd0.len() as u32 + 4;
        let mut data_at = ifd0_at + ifd0_len;
        let ifd0_bytes = encode_ifd(&ifd0, &mut data_at);
        let exif_at = exif_ifd_at.unwrap_or(data_at).max(data_at);
        out.extend_from_slice(&ifd0_bytes.0);
        out.extend_from_slice(&ifd0_bytes.1);
        // Patch the Exif pointer value.
        let ptr_entry = 8 + 2 + 12 * (ifd0.len() - 1) + 8;
        out[ptr_entry..ptr_entry + 4].copy_from_slice(&exif_at.to_le_bytes());
        out.resize(exif_at as usize, 0);
        let mut data_at = exif_at + 2 + 12 * ex.len() as u32 + 4;
        let ex_bytes = encode_ifd(&ex, &mut data_at);
        out.extend_from_slice(&ex_bytes.0);
        out.extend_from_slice(&ex_bytes.1);
        out
    }

    /// (IFD bytes, out-of-line value bytes placed right after it).
    fn encode_ifd(entries: &[(u16, V)], data_at: &mut u32) -> (Vec<u8>, Vec<u8>) {
        let mut ifd = (entries.len() as u16).to_le_bytes().to_vec();
        let mut data = Vec::new();
        for (tag, v) in entries {
            ifd.extend_from_slice(&tag.to_le_bytes());
            let (typ, count, bytes): (u16, u32, Vec<u8>) = match v {
                V::Ascii(s) => {
                    let mut b = s.as_bytes().to_vec();
                    b.push(0);
                    (2, b.len() as u32, b)
                }
                V::Short(x) => (3, 1, x.to_le_bytes().to_vec()),
                V::Long(x) => (4, 1, x.to_le_bytes().to_vec()),
                V::Rational(n, d) => {
                    let mut b = n.to_le_bytes().to_vec();
                    b.extend_from_slice(&d.to_le_bytes());
                    (5, 1, b)
                }
            };
            ifd.extend_from_slice(&typ.to_le_bytes());
            ifd.extend_from_slice(&count.to_le_bytes());
            if bytes.len() <= 4 {
                let mut b = bytes.clone();
                b.resize(4, 0);
                ifd.extend_from_slice(&b);
            } else {
                ifd.extend_from_slice(&(*data_at + data.len() as u32).to_le_bytes());
                data.extend_from_slice(&bytes);
                if data.len() % 2 == 1 {
                    data.push(0);
                }
            }
        }
        ifd.extend_from_slice(&0u32.to_le_bytes()); // no next IFD
        *data_at += data.len() as u32;
        (ifd, data)
    }

    /// A JPEG with only SOI, APP1 Exif, a dummy scan and EOI (no real pixels).
    pub fn jpeg(s: &Shot) -> Vec<u8> {
        let t = tiff(s, None);
        let mut out = vec![0xFF, 0xD8];
        // An APP0 first, as real files have.
        out.extend_from_slice(&[0xFF, 0xE0, 0x00, 0x10]);
        out.extend_from_slice(b"JFIF\0\x01\x01\0\0\x01\0\x01\0\0");
        out.extend_from_slice(&[0xFF, 0xE1]);
        out.extend_from_slice(&((t.len() + 8) as u16).to_be_bytes());
        out.extend_from_slice(b"Exif\0\0");
        out.extend_from_slice(&t);
        out.extend_from_slice(&[0xFF, 0xDA, 0x00, 0x02, 0x00, 0x00, 0xFF, 0xD9]);
        out
    }

    fn fbox(t: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut b = ((body.len() + 8) as u32).to_be_bytes().to_vec();
        b.extend_from_slice(t);
        b.extend_from_slice(body);
        b
    }

    /// A HEIC: ftyp, meta (iinf with one Exif item, iloc pointing into mdat),
    /// mdat holding the Exif payload after `mdat_padding` bytes of "pixels".
    pub fn heic(s: &Shot, mdat_padding: usize) -> Vec<u8> {
        let mut payload = 6u32.to_be_bytes().to_vec();
        payload.extend_from_slice(b"Exif\0\0");
        payload.extend_from_slice(&tiff(s, None));
        let ftyp = fbox(b"ftyp", b"heic\0\0\0\0mif1heic");
        let infe = fbox(b"infe", b"\x02\0\0\0\0\x01\0\0Exif");
        let mut iinf_body = b"\0\0\0\0\0\x01".to_vec();
        iinf_body.extend_from_slice(&infe);
        let iinf = fbox(b"iinf", &iinf_body);
        let iloc_len = 8 + 4 + 2 + 2 + 2 + 2 + 2 + 2 + 4 + 4;
        let meta_len = 8 + 4 + iloc_len + iinf.len();
        let exif_off = (ftyp.len() + meta_len + 8 + mdat_padding) as u32;
        let mut iloc_body = b"\x01\0\0\0\x44\x00\0\x01\0\x01\0\0\0\0\0\x01".to_vec();
        iloc_body.extend_from_slice(&exif_off.to_be_bytes());
        iloc_body.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        let iloc = fbox(b"iloc", &iloc_body);
        let mut meta_body = b"\0\0\0\0".to_vec();
        meta_body.extend_from_slice(&iloc);
        meta_body.extend_from_slice(&iinf);
        let meta = fbox(b"meta", &meta_body);
        assert_eq!(meta.len(), meta_len);
        let mut mdat_body = vec![0u8; mdat_padding];
        mdat_body.extend_from_slice(&payload);
        let mut out = ftyp;
        out.extend_from_slice(&meta);
        out.extend_from_slice(&fbox(b"mdat", &mdat_body));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::fixture::*;
    use super::*;
    use std::io::Cursor;

    fn a7() -> Shot {
        Shot {
            make: Some("SONY"),
            model: Some("ILCE-7M4"),
            taken: Some("2026:11:14 18:05:09"),
            subsec: Some("42"),
            offset: Some("+08:00"),
            lens: Some("FE 35mm F1.8"),
            focal: Some((350, 10)),
            f_number: Some((18, 10)),
            exposure: Some((1, 250)),
            iso: Some(400),
            ..Default::default()
        }
    }

    fn read(bytes: &[u8]) -> (Result<PhotoMeta, MetaError>, u64) {
        let mut r = Capped::new(Cursor::new(bytes), HARD_CAP);
        let m = read_meta(&mut r, bytes.len() as u64);
        (m, r.read)
    }

    fn check(m: &PhotoMeta) {
        assert_eq!(m.taken.unwrap().to_string(), "2026-11-14 18:05:09");
        assert_eq!(m.subsec_ms(), 420);
        assert_eq!(m.offset_min, Some(480));
        assert_eq!(m.make.as_deref(), Some("SONY"));
        assert_eq!(m.model.as_deref(), Some("ILCE-7M4"));
        assert_eq!(m.lens.as_deref(), Some("FE 35mm F1.8"));
        assert_eq!(m.focal_mm, Some(35.0));
        assert_eq!(m.f_number, Some(1.8));
        assert_eq!(m.exposure_s, Some(0.004));
        assert_eq!(m.iso, Some(400));
    }

    #[test]
    fn jpeg_tiff_heic_same_fields_and_id() {
        let s = a7();
        let mut ids = Vec::new();
        for bytes in [jpeg(&s), tiff(&s, None), heic(&s, 100_000)] {
            let (m, n) = read(&bytes);
            let m = m.unwrap();
            check(&m);
            ids.push(m.shot_id().unwrap());
            assert!(n < 20_000, "read {n} bytes");
        }
        assert!(ids.windows(2).all(|w| w[0] == w[1]));
    }

    #[test]
    fn offsets_parse() {
        assert_eq!(parse_offset("-05:30"), Some(-330));
        assert_eq!(parse_offset("Z"), Some(0));
        assert_eq!(parse_offset("+25:00"), None);
        assert_eq!(parse_offset("   :  "), None);
    }

    #[test]
    fn no_date_and_unsupported() {
        let s = Shot {
            taken: None,
            ..a7()
        };
        let (m, _) = read(&jpeg(&s));
        assert!(m.unwrap().shot_id().is_none());
        assert!(matches!(read(b"GIF89a....").0, Err(MetaError::Unsupported)));
        // JPEG without APP1: stops at the scan.
        let (m, _) = read(&[0xFF, 0xD8, 0xFF, 0xDA, 0, 2, 0, 0]);
        assert!(matches!(m, Err(MetaError::NoExif)));
    }
}
