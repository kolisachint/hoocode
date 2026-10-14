//! Pixel-dimension sniffing for PNG/JPEG/GIF/WebP images from base64 data,
//! ported from `terminal-image.ts`'s `get*Dimensions` functions.

use base64::Engine;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImageDimensions {
    pub width_px: u32,
    pub height_px: u32,
}

/// Base64 characters decoded for the fixed-position headers: 64 characters
/// are 48 bytes, past the 30 bytes a WebP VP8X header reads.
const HEADER_CHARS: usize = 64;

/// Base64 characters decoded first for a JPEG marker scan. Usually the frame
/// header is inside this; when it is not, the scan retries on the whole input.
const JPEG_SCAN_CHARS: usize = 128 * 1024;

fn decode(base64_data: &str) -> Option<Vec<u8>> {
    base64::engine::general_purpose::STANDARD
        .decode(base64_data)
        .ok()
}

/// Decode at most `max_chars` characters. The bool is true when the input was
/// longer, so the prefix holds only whole 4-character groups.
fn decode_prefix(base64_data: &str, max_chars: usize) -> Option<(Vec<u8>, bool)> {
    if base64_data.len() <= max_chars {
        return Some((decode(base64_data)?, false));
    }
    let take = max_chars / 4 * 4;
    let buf = base64::engine::general_purpose::STANDARD
        .decode(&base64_data.as_bytes()[..take])
        .ok()?;
    Some((buf, true))
}

pub fn get_png_dimensions(base64_data: &str) -> Option<ImageDimensions> {
    let buf = decode_prefix(base64_data, HEADER_CHARS)?.0;
    if buf.len() < 24 {
        return None;
    }
    if buf[0..4] != [0x89, 0x50, 0x4e, 0x47] {
        return None;
    }
    let width = u32::from_be_bytes(buf[16..20].try_into().ok()?);
    let height = u32::from_be_bytes(buf[20..24].try_into().ok()?);
    Some(ImageDimensions {
        width_px: width,
        height_px: height,
    })
}

pub fn get_jpeg_dimensions(base64_data: &str) -> Option<ImageDimensions> {
    if let Some((buf, cut)) = decode_prefix(base64_data, JPEG_SCAN_CHARS) {
        match scan_jpeg(&buf) {
            Some(dims) => return Some(dims),
            None if !cut => return None,
            None => {}
        }
    }
    // The frame header is past the prefix: scan the whole input.
    scan_jpeg(&decode(base64_data)?)
}

fn scan_jpeg(buf: &[u8]) -> Option<ImageDimensions> {
    if buf.len() < 2 {
        return None;
    }
    if buf[0] != 0xff || buf[1] != 0xd8 {
        return None;
    }

    let mut offset = 2usize;
    while offset < buf.len().saturating_sub(9) {
        if buf[offset] != 0xff {
            offset += 1;
            continue;
        }
        let marker = buf[offset + 1];
        if (0xc0..=0xc2).contains(&marker) {
            let height = u16::from_be_bytes(buf[offset + 5..offset + 7].try_into().ok()?);
            let width = u16::from_be_bytes(buf[offset + 7..offset + 9].try_into().ok()?);
            return Some(ImageDimensions {
                width_px: width as u32,
                height_px: height as u32,
            });
        }
        if offset + 3 >= buf.len() {
            return None;
        }
        let length = u16::from_be_bytes(buf[offset + 2..offset + 4].try_into().ok()?) as usize;
        if length < 2 {
            return None;
        }
        offset += 2 + length;
    }
    None
}

pub fn get_gif_dimensions(base64_data: &str) -> Option<ImageDimensions> {
    let buf = decode_prefix(base64_data, HEADER_CHARS)?.0;
    if buf.len() < 10 {
        return None;
    }
    let sig = std::str::from_utf8(&buf[0..6]).ok()?;
    if sig != "GIF87a" && sig != "GIF89a" {
        return None;
    }
    let width = u16::from_le_bytes(buf[6..8].try_into().ok()?);
    let height = u16::from_le_bytes(buf[8..10].try_into().ok()?);
    Some(ImageDimensions {
        width_px: width as u32,
        height_px: height as u32,
    })
}

pub fn get_webp_dimensions(base64_data: &str) -> Option<ImageDimensions> {
    let buf = decode_prefix(base64_data, HEADER_CHARS)?.0;
    if buf.len() < 30 {
        return None;
    }
    let riff = std::str::from_utf8(&buf[0..4]).ok()?;
    let webp = std::str::from_utf8(&buf[8..12]).ok()?;
    if riff != "RIFF" || webp != "WEBP" {
        return None;
    }
    let chunk = std::str::from_utf8(&buf[12..16]).ok()?;
    match chunk {
        "VP8 " => {
            let width = u16::from_le_bytes(buf[26..28].try_into().ok()?) & 0x3fff;
            let height = u16::from_le_bytes(buf[28..30].try_into().ok()?) & 0x3fff;
            Some(ImageDimensions {
                width_px: width as u32,
                height_px: height as u32,
            })
        }
        "VP8L" => {
            if buf.len() < 25 {
                return None;
            }
            let bits = u32::from_le_bytes(buf[21..25].try_into().ok()?);
            let width = (bits & 0x3fff) + 1;
            let height = ((bits >> 14) & 0x3fff) + 1;
            Some(ImageDimensions {
                width_px: width,
                height_px: height,
            })
        }
        "VP8X" => {
            let width = (buf[24] as u32 | (buf[25] as u32) << 8 | (buf[26] as u32) << 16) + 1;
            let height = (buf[27] as u32 | (buf[28] as u32) << 8 | (buf[29] as u32) << 16) + 1;
            Some(ImageDimensions {
                width_px: width,
                height_px: height,
            })
        }
        _ => None,
    }
}

pub fn get_image_dimensions(base64_data: &str, mime_type: &str) -> Option<ImageDimensions> {
    match mime_type {
        "image/png" => get_png_dimensions(base64_data),
        "image/jpeg" => get_jpeg_dimensions(base64_data),
        "image/gif" => get_gif_dimensions(base64_data),
        "image/webp" => get_webp_dimensions(base64_data),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;

    fn b64(bytes: &[u8]) -> String {
        base64::engine::general_purpose::STANDARD.encode(bytes)
    }

    #[test]
    fn png_dimensions() {
        let mut buf = vec![0x89, 0x50, 0x4e, 0x47, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        buf.extend_from_slice(&100u32.to_be_bytes());
        buf.extend_from_slice(&200u32.to_be_bytes());
        let dims = get_png_dimensions(&b64(&buf)).unwrap();
        assert_eq!(dims.width_px, 100);
        assert_eq!(dims.height_px, 200);
    }

    #[test]
    fn png_dimensions_rejects_bad_signature() {
        let buf = vec![0u8; 24];
        assert!(get_png_dimensions(&b64(&buf)).is_none());
    }

    #[test]
    fn gif_dimensions() {
        let mut buf = b"GIF89a".to_vec();
        buf.extend_from_slice(&320u16.to_le_bytes());
        buf.extend_from_slice(&240u16.to_le_bytes());
        buf.extend_from_slice(&[0u8; 2]);
        let dims = get_gif_dimensions(&b64(&buf)).unwrap();
        assert_eq!(dims.width_px, 320);
        assert_eq!(dims.height_px, 240);
    }

    #[test]
    fn webp_vp8x_dimensions() {
        let mut buf = vec![0u8; 30];
        buf[0..4].copy_from_slice(b"RIFF");
        buf[8..12].copy_from_slice(b"WEBP");
        buf[12..16].copy_from_slice(b"VP8X");
        // width-1 = 99 (little endian 24-bit), height-1 = 199
        buf[24] = 99;
        buf[25] = 0;
        buf[26] = 0;
        buf[27] = 199;
        buf[28] = 0;
        buf[29] = 0;
        let dims = get_webp_dimensions(&b64(&buf)).unwrap();
        assert_eq!(dims.width_px, 100);
        assert_eq!(dims.height_px, 200);
    }

    /// A SOF0 segment (frame header), without SOI.
    fn jpeg_frame(width: u16, height: u16) -> Vec<u8> {
        let mut buf = vec![0xff, 0xc0, 0, 0x11, 8];
        buf.extend_from_slice(&height.to_be_bytes());
        buf.extend_from_slice(&width.to_be_bytes());
        buf.extend_from_slice(&[3; 12]);
        buf
    }

    /// An APP0 segment whose length field is 0xffff (65535 bytes including the length).
    fn max_app_segment() -> Vec<u8> {
        let mut seg = vec![0xff, 0xe0, 0xff, 0xff];
        seg.resize(2 + 65535, 0);
        seg
    }

    #[test]
    fn jpeg_dimensions_from_the_frame_header() {
        let mut buf = vec![0xff, 0xd8];
        buf.extend(jpeg_frame(200, 300));
        let dims = get_jpeg_dimensions(&b64(&buf)).unwrap();
        assert_eq!(dims.width_px, 200);
        assert_eq!(dims.height_px, 300);
    }

    #[test]
    fn jpeg_dimensions_past_the_scan_prefix() {
        // Two maximal APP segments push the frame header past the first decoded
        // prefix, so the scan falls back to the whole input.
        let mut buf = vec![0xff, 0xd8];
        buf.extend(max_app_segment());
        buf.extend(max_app_segment());
        buf.extend(jpeg_frame(640, 480));
        assert!(buf.len() > JPEG_SCAN_CHARS / 4 * 3);
        let dims = get_jpeg_dimensions(&b64(&buf)).unwrap();
        assert_eq!(dims.width_px, 640);
        assert_eq!(dims.height_px, 480);
    }

    #[test]
    fn png_dimensions_ignore_the_payload_after_the_header() {
        let mut buf = vec![0x89, 0x50, 0x4e, 0x47, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        buf.extend_from_slice(&100u32.to_be_bytes());
        buf.extend_from_slice(&200u32.to_be_bytes());
        buf.extend((0..200_000u32).map(|i| (i % 251) as u8));
        let dims = get_png_dimensions(&b64(&buf)).unwrap();
        assert_eq!(dims.width_px, 100);
        assert_eq!(dims.height_px, 200);
    }

    #[test]
    fn decode_prefix_is_the_start_of_the_whole_decode() {
        let bytes: Vec<u8> = (0..1000u32).map(|i| (i * 7) as u8).collect();
        let data = b64(&bytes);
        let (prefix, cut) = decode_prefix(&data, HEADER_CHARS).unwrap();
        assert!(cut);
        assert_eq!(prefix, bytes[..HEADER_CHARS / 4 * 3]);
        let (whole, cut) = decode_prefix(&data, usize::MAX).unwrap();
        assert!(!cut);
        assert_eq!(whole, bytes);
    }

    #[test]
    fn image_dimensions_dispatches_by_mime_type() {
        let mut buf = vec![0x89, 0x50, 0x4e, 0x47, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        buf.extend_from_slice(&10u32.to_be_bytes());
        buf.extend_from_slice(&20u32.to_be_bytes());
        let data = b64(&buf);
        assert!(get_image_dimensions(&data, "image/png").is_some());
        assert!(get_image_dimensions(&data, "image/tiff").is_none());
    }
}
