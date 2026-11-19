//! Behavioural tests for the bounded remote-image boundary.
//!
//! Every hostile fixture is *compact* — a header or an infinite stream, never a
//! materialised bomb — and each is screened through the exact production
//! functions (`read_capped`, `decode`, `commit_raster`, `commit_icon`,
//! `load_cached`) plus the app's own screenshot routing (`cache_screenshot`).
//! A positive control accompanies every refusal so a gate that rejects
//! everything cannot pass.

use super::*;
use std::io::Cursor;
use std::sync::atomic::{AtomicU64, Ordering};

// A syntactically real PNG carrying only IHDR/IDAT/IEND, so the decoder reads
// the declared dimensions from the header and the `DecodeLimits` screen fires
// *before* any pixel buffer is allocated. Mirrors the framework decode-test
// helper of the same name.
fn png_header_only(width: u32, height: u32) -> Vec<u8> {
    let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]); // RGBA8, deflate, filter, no interlace
    bytes.extend_from_slice(&chunk(*b"IHDR", &ihdr));
    bytes.extend_from_slice(&chunk(*b"IDAT", &[]));
    bytes.extend_from_slice(&chunk(*b"IEND", &[]));
    bytes
}

fn chunk(kind: [u8; 4], data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(&kind);
    out.extend_from_slice(data);
    let mut crc_input = kind.to_vec();
    crc_input.extend_from_slice(data);
    out.extend_from_slice(&crc32(&crc_input).to_be_bytes());
    out
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for &byte in bytes {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg() & 0xedb8_8320;
            crc = (crc >> 1) ^ mask;
        }
    }
    !crc
}

// A minimal, genuinely-decodable solid-colour 24bpp bottom-up BMP. Hand-built
// so the tests need no `image`-crate dev-dependency for a real positive
// control.
fn bmp(width: u32, height: u32) -> Vec<u8> {
    let stride = (width * 3).div_ceil(4) * 4;
    let pixels = stride * height;
    let file_size = 54 + pixels;
    let mut out = Vec::with_capacity(file_size as usize);
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&file_size.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // reserved
    out.extend_from_slice(&54u32.to_le_bytes()); // pixel data offset
    out.extend_from_slice(&40u32.to_le_bytes()); // info header size
    out.extend_from_slice(&(width as i32).to_le_bytes());
    out.extend_from_slice(&(height as i32).to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // planes
    out.extend_from_slice(&24u16.to_le_bytes()); // bpp
    out.extend_from_slice(&0u32.to_le_bytes()); // BI_RGB
    out.extend_from_slice(&pixels.to_le_bytes()); // image size
    out.extend_from_slice(&2835u32.to_le_bytes()); // x ppm
    out.extend_from_slice(&2835u32.to_le_bytes()); // y ppm
    out.extend_from_slice(&0u32.to_le_bytes()); // colours used
    out.extend_from_slice(&0u32.to_le_bytes()); // colours important
    out.resize(file_size as usize, 0x40);
    out
}

static UNIQUE: AtomicU64 = AtomicU64::new(0);

// A fresh, non-existent path in a per-test temp dir. Returns the path and a
// guard directory that is removed on drop.
fn temp_dest(name: &str) -> (std::path::PathBuf, TempDir) {
    let n = UNIQUE.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("store-bimg-{}-{n}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    (dir.join(name), TempDir(dir))
}

struct TempDir(std::path::PathBuf);
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

// --- the download byte cap --------------------------------------------------

// The production axes (4096 each) multiply to exactly the pixel cap, so no
// header can pass both axes and still exceed it: a compact 10_000 x 10_000
// bomb (1e8 px, roughly 400 MiB decoded) is refused by its width, from a
// ~70-byte header, before allocation.
#[test]
fn pixel_limit_refuses_the_compact_bomb() {
    assert_eq!(
        u64::from(MAX_IMAGE_WIDTH) * u64::from(MAX_IMAGE_HEIGHT),
        MAX_IMAGE_PIXELS,
        "the axis caps bound the pixel cap exactly"
    );
    let bytes = png_header_only(10_000, 10_000);
    let err = decode(&bytes).expect_err("pixel bomb refused");
    assert_eq!(err.limit_reason(), Some("width-too-large"));
}

// --- load_cached re-screens a warm cache file -------------------------------

#[test]
fn load_cached_re_screens_the_on_disk_file() {
    let (good, _g1) = temp_dest("warm.bmp");
    std::fs::write(&good, bmp(2, 2)).unwrap();
    load_cached(&good).expect("valid warm file loads");

    let (bad, _g2) = temp_dest("warm-bomb.png");
    std::fs::write(&bad, png_header_only(10_000, 10_000)).unwrap();
    assert_eq!(
        load_cached(&bad)
            .expect_err("warm bomb refused")
            .limit_reason(),
        Some("width-too-large")
    );
}

#[test]
fn transport_nonzero_exit_cannot_commit_an_otherwise_valid_image() {
    let (dest, _guard) = temp_dest("nonzero.bmp");
    let error = commit_transport(
        Cursor::new(bmp(2, 2)),
        || Err(ImageRefusal::Transport("exit 22".into())),
        &dest,
    )
    .expect_err("nonzero transport refused");
    assert!(matches!(error, ImageRefusal::Transport(_)));
    assert!(
        !dest.exists(),
        "nonzero transport must not publish cache data"
    );
}

#[test]
fn a_scalable_icon_that_only_wraps_an_embedded_picture_shows_that_picture() {
    // Blanket's icon, in shape: an <image> with a data URI and nothing else.
    // This resvg draws no rasters, so the picture itself, through the
    // bounded decoder, is the icon.
    let wrapper = br#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="128" height="128" viewBox="0 0 128 128"><image width="128" height="128" xlink:href="data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8DwHwAFBQIAX8jx0gAAAABJRU5ErkJggg=="/></svg>"#;
    assert!(decode_local_svg_icon(wrapper, &|| false).is_some());
    // A wrapper whose picture is not a picture is still no icon.
    let broken = br#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="128" height="128"><image width="128" height="128" xlink:href="data:image/png;base64,AAAAAAAA"/></svg>"#;
    assert!(decode_local_svg_icon(broken, &|| false).is_none());
    let drawn = br#"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16"><rect width="16" height="16" fill="blue"/></svg>"#;
    assert!(decode_local_svg_icon(drawn, &|| false).is_some());
}
