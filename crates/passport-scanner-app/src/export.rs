//! Lossless four-page document export. JPEG streams are embedded, not recompressed.
//!
//! PDF image dictionaries and DCTDecode follow Adobe PDF Reference 1.6 §4.8:
//! https://opensource.adobe.com/dc-acrobat-sdk-docs/pdfstandards/pdfreference1.6.pdf

use base64::{Engine as _, engine::general_purpose::STANDARD};
use image::{ImageDecoder, codecs::jpeg::JpegDecoder, metadata::Orientation};
use passport_scanner_core::{Error, Result};
use passport_scanner_protocol::{Scan, Side, Source};
use std::{
    io::{Cursor, Write},
    path::Path,
};

const MAX_JPEG_BYTES: usize = 32 * 1024 * 1024;
const MAX_PIXELS: u64 = 40_000_000;

/// Export White front, White back, Infrared front, Infrared back, in that order.
///
/// Input follows protocol State order (newest first); the latest capture of each side is used. Every required image must be a
/// local base64 JPEG; missing sides/images and invalid data fail before the
/// destination is opened. No URLs are fetched. Pixel dimensions come from the
/// JPEG, and EXIF orientation is represented by the PDF placement matrix.
pub fn write_document_pdf(scans: &[Scan], document_id: &str, path: &Path) -> Result<()> {
    let bytes = document_pdf(scans, document_id)?;
    std::fs::write(path, bytes)?;
    Ok(())
}

/// Render in memory for an HTTP download, with the same validation as file export.
pub fn document_pdf(scans: &[Scan], document_id: &str) -> Result<Vec<u8>> {
    if document_id.is_empty() {
        return Err(Error::Malformed("A document ID is required for PDF export".into()));
    }
    let mut pages = Vec::with_capacity(4);
    for (source, side, label) in [
        (Source::White, Side::Front, "White - front"),
        (Source::White, Side::Back, "White - back"),
        (Source::Infrared, Side::Front, "Infrared - front"),
        (Source::Infrared, Side::Back, "Infrared - back"),
    ] {
        let scan = scans.iter().find(|s| s.document == document_id && s.side == side).ok_or_else(
            || Error::Malformed(format!("PDF requires all four images; missing {label} side")),
        )?;
        let image = scan.images.iter().find(|image| image.source == source).ok_or_else(|| {
            Error::Malformed(format!("PDF requires all four images; missing {label} image"))
        })?;
        pages.push(JpegPage::read(&image.url, label)?);
    }
    build_pdf(&pages)
}

struct JpegPage {
    bytes: Vec<u8>,
    width: u32,
    height: u32,
    components: u8,
    orientation: Orientation,
    label: &'static str,
}

impl JpegPage {
    fn read(url: &str, label: &'static str) -> Result<Self> {
        let encoded = url.strip_prefix("data:image/jpeg;base64,").ok_or_else(|| {
            Error::Malformed(format!(
                "{label}: only local data:image/jpeg;base64 images are supported"
            ))
        })?;
        if encoded.len() > MAX_JPEG_BYTES * 4 / 3 + 4 {
            return Err(Error::Malformed(format!("{label}: JPEG exceeds the export size limit")));
        }
        let bytes = STANDARD
            .decode(encoded)
            .map_err(|e| Error::Malformed(format!("{label}: invalid JPEG base64: {e}")))?;
        let components = jpeg_components(&bytes)?;
        let mut decoder = JpegDecoder::new(Cursor::new(&bytes))
            .map_err(|e| Error::Malformed(format!("{label}: invalid JPEG: {e}")))?;
        let (width, height) = decoder.dimensions();
        if width == 0 || height == 0 || u64::from(width) * u64::from(height) > MAX_PIXELS {
            return Err(Error::Malformed(format!("{label}: invalid or excessive JPEG dimensions")));
        }
        let orientation = decoder.orientation().map_err(Error::backend)?;
        // Validate the compressed data as well as its headers. These pixels are
        // discarded; the original compressed bytes are the PDF image stream.
        let mut pixels = vec![0; usize::try_from(decoder.total_bytes()).map_err(Error::backend)?];
        decoder
            .read_image(&mut pixels)
            .map_err(|e| Error::Malformed(format!("{label}: invalid JPEG image data: {e}")))?;
        Ok(Self { bytes, width, height, components, orientation, label })
    }
}

// Inspect the original JPEG color components: decoders may convert CMYK to RGB,
// which would otherwise give the embedded, unconverted DCT stream a wrong color space.
fn jpeg_components(bytes: &[u8]) -> Result<u8> {
    if !bytes.starts_with(&[0xFF, 0xD8]) || !bytes.ends_with(&[0xFF, 0xD9]) {
        return Err(Error::Malformed("JPEG must have complete SOI/EOI markers".into()));
    }
    let mut at = 2;
    while at + 4 <= bytes.len() {
        if bytes[at] != 0xFF {
            break;
        }
        while at < bytes.len() && bytes[at] == 0xFF {
            at += 1;
        }
        let Some(&marker) = bytes.get(at) else {
            break;
        };
        at += 1;
        if marker == 0xDA || marker == 0xD9 {
            break;
        }
        if marker == 0x01 || (0xD0..=0xD8).contains(&marker) {
            continue;
        }
        let Some(length_bytes) = bytes.get(at..at + 2) else {
            break;
        };
        let length = usize::from(u16::from_be_bytes([length_bytes[0], length_bytes[1]]));
        if length < 2 || at + length > bytes.len() {
            break;
        }
        if matches!(marker, 0xC0..=0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF) {
            if !matches!(marker, 0xC0..=0xC2) || length < 8 || bytes[at + 2] != 8 {
                return Err(Error::Unsupported(
                    "PDF export requires an 8-bit baseline or progressive JPEG".into(),
                ));
            }
            let count = bytes[at + 7];
            return if matches!(count, 1 | 3) {
                Ok(count)
            } else {
                Err(Error::Unsupported("PDF export supports grayscale and RGB/YCbCr JPEGs".into()))
            };
        }
        at += length;
    }
    Err(Error::Malformed("JPEG has no valid frame header".into()))
}

fn placement(page: &JpegPage) -> (f64, f64, [f64; 6]) {
    use Orientation::*;
    let swapped = matches!(page.orientation, Rotate90 | Rotate270 | Rotate90FlipH | Rotate270FlipH);
    let (iw, ih) = if swapped { (page.height, page.width) } else { (page.width, page.height) };
    // A4 in the image's orientation, with a readable label below the image.
    let (pw, ph): (f64, f64) = if iw > ih { (841.89, 595.28) } else { (595.28, 841.89) };
    let scale = ((pw - 72.0) / f64::from(iw)).min((ph - 108.0) / f64::from(ih));
    let (w, h) = (f64::from(iw) * scale, f64::from(ih) * scale);
    let (x, y) = ((pw - w) / 2.0, 72.0 + (ph - 108.0 - h) / 2.0);
    let matrix = match page.orientation {
        NoTransforms => [w, 0.0, 0.0, h, x, y],
        FlipHorizontal => [-w, 0.0, 0.0, h, x + w, y],
        Rotate180 => [-w, 0.0, 0.0, -h, x + w, y + h],
        FlipVertical => [w, 0.0, 0.0, -h, x, y + h],
        Rotate90 => [0.0, -h, w, 0.0, x, y + h],
        Rotate270 => [0.0, h, -w, 0.0, x + w, y],
        Rotate90FlipH => [0.0, -h, -w, 0.0, x + w, y + h],
        Rotate270FlipH => [0.0, h, w, 0.0, x, y],
    };
    (pw, ph, matrix)
}

fn stream(dictionary: &str, bytes: &[u8]) -> Vec<u8> {
    let mut object = format!("<< {dictionary} /Length {} >>\nstream\n", bytes.len()).into_bytes();
    object.extend_from_slice(bytes);
    object.extend_from_slice(b"\nendstream");
    object
}

fn build_pdf(pages: &[JpegPage]) -> Result<Vec<u8>> {
    let kids = (0..pages.len()).map(|n| format!("{} 0 R", 4 + n * 3)).collect::<Vec<_>>().join(" ");
    let mut objects = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        format!("<< /Type /Pages /Count {} /Kids [{kids}] >>", pages.len()).into_bytes(),
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec(),
    ];
    for (index, page) in pages.iter().enumerate() {
        let image_id = 5 + index * 3;
        let content_id = image_id + 1;
        let (pw, ph, [a, b, c, d, e, f]) = placement(page);
        objects.push(format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {pw:.2} {ph:.2}] /Resources << /Font << /F1 3 0 R >> /XObject << /Im0 {image_id} 0 R >> >> /Contents {content_id} 0 R >>").into_bytes());
        let color = if page.components == 1 { "/DeviceGray" } else { "/DeviceRGB" };
        objects.push(stream(&format!("/Type /XObject /Subtype /Image /Width {} /Height {} /ColorSpace {color} /BitsPerComponent 8 /Filter /DCTDecode", page.width, page.height), &page.bytes));
        let content = format!(
            "q\n{a:.6} {b:.6} {c:.6} {d:.6} {e:.6} {f:.6} cm\n/Im0 Do\nQ\nBT /F1 14 Tf 36 36 Td ({} - page {} of {}) Tj ET\n",
            page.label,
            index + 1,
            pages.len()
        );
        objects.push(stream("", content.as_bytes()));
    }
    let mut pdf = b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n".to_vec();
    let mut offsets = Vec::with_capacity(objects.len());
    for (index, object) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        writeln!(pdf, "{} 0 obj", index + 1)?;
        pdf.extend_from_slice(object);
        pdf.extend_from_slice(b"\nendobj\n");
    }
    let xref = pdf.len();
    writeln!(pdf, "xref\n0 {}\n0000000000 65535 f ", objects.len() + 1)?;
    for offset in offsets {
        writeln!(pdf, "{offset:010} 00000 n ")?;
    }
    write!(
        pdf,
        "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
        objects.len() + 1
    )?;
    Ok(pdf)
}

#[cfg(test)]
mod tests {
    use super::*;
    use passport_scanner_protocol::Image;

    fn jpeg(color: [u8; 3], width: u32, height: u32) -> Vec<u8> {
        let pixels = image::RgbImage::from_pixel(width, height, image::Rgb(color));
        let mut bytes = Vec::new();
        image::codecs::jpeg::JpegEncoder::new(&mut bytes).encode_image(&pixels).unwrap();
        bytes
    }

    fn fixture() -> Vec<Scan> {
        [Side::Front, Side::Back]
            .into_iter()
            .enumerate()
            .map(|(n, side)| Scan {
                id: format!("synthetic-{n}"),
                document: "synthetic".into(),
                side,
                at: "2026-01-01T00:00:00Z".into(),
                format: None,
                mrz: vec![],
                mrz_check: None,
                fields: vec![],
                images: [Source::White, Source::Infrared]
                    .into_iter()
                    .enumerate()
                    .map(|(i, source)| Image {
                        source,
                        url: format!(
                            "data:image/jpeg;base64,{}",
                            STANDARD.encode(jpeg(
                                [20 + n as u8 * 50, 40 + i as u8 * 50, 100],
                                13 + n as u32,
                                9 + i as u32
                            ))
                        ),
                        width: 999,
                        height: 999, // JPEG dimensions are authoritative.
                    })
                    .collect(),
            })
            .collect()
    }

    #[test]
    fn pdf_has_four_ordered_pages_and_exact_jpeg_streams() {
        let scans = fixture();
        let pdf = document_pdf(&scans, "synthetic").unwrap();
        let text = String::from_utf8_lossy(&pdf);
        assert!(text.contains("/Count 4 /Kids [4 0 R 7 0 R 10 0 R 13 0 R]"));
        let mut position = 0;
        for (side, source, label) in [
            (0, 0, "White - front"),
            (1, 0, "White - back"),
            (0, 1, "Infrared - front"),
            (1, 1, "Infrared - back"),
        ] {
            let bytes =
                STANDARD.decode(scans[side].images[source].url.split_once(',').unwrap().1).unwrap();
            let found = pdf.windows(bytes.len()).position(|w| w == bytes).unwrap();
            assert!(found > position);
            position = found;
            assert!(text.contains(label));
            assert!(text.contains(&format!("/Width {} /Height {}", 13 + side, 9 + source)));
        }
        // Resolve every xref offset to its object header, not a lossy text offset.
        let marker = b"startxref\n";
        let at = pdf.windows(marker.len()).rposition(|w| w == marker).unwrap() + marker.len();
        let xref: usize =
            std::str::from_utf8(&pdf[at..]).unwrap().lines().next().unwrap().parse().unwrap();
        let lines: Vec<_> = std::str::from_utf8(&pdf[xref..]).unwrap().lines().collect();
        assert_eq!(lines[0], "xref");
        for (i, line) in lines[3..18].iter().enumerate() {
            let offset: usize = line[..10].parse().unwrap();
            assert!(pdf[offset..].starts_with(format!("{} 0 obj\n", i + 1).as_bytes()));
        }
        // Optional artifact enables an independent PDF parser/render check.
        if let Some(path) = std::env::var_os("PASSPORT_SCANNER_SYNTHETIC_PDF") {
            std::fs::write(path, pdf).unwrap();
        }
    }

    #[test]
    fn missing_side_or_source_fails_without_overwriting_a_file() {
        let mut scans = fixture();
        assert!(
            document_pdf(&scans[..1], "synthetic")
                .unwrap_err()
                .to_string()
                .contains("White - back")
        );
        scans[1].images.pop();
        assert!(
            document_pdf(&scans, "synthetic").unwrap_err().to_string().contains("Infrared - back")
        );
        let path = std::env::temp_dir()
            .join(format!("passport-scanner-pdf-test-{}.pdf", std::process::id()));
        std::fs::write(&path, b"preserve").unwrap();
        assert!(write_document_pdf(&scans, "synthetic", &path).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"preserve");
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn rejects_remote_urls_bad_base64_and_truncated_jpeg() {
        let mut scans = fixture();
        for url in [
            "https://example.invalid/image.jpg",
            "data:image/jpeg;base64,!",
            "data:image/jpeg;base64,/9j/",
        ] {
            scans[0].images[0].url = url.into();
            assert!(document_pdf(&scans, "synthetic").is_err());
        }
    }

    #[test]
    fn exif_rotation_changes_layout_without_recompressing_pixels() {
        let original = jpeg([10, 60, 120], 12, 8);
        let mut exif =
            b"Exif\0\0II\x2A\0\x08\0\0\0\x01\0\x12\x01\x03\0\x01\0\0\0\x06\0\0\0\0\0\0\0".to_vec();
        let mut oriented = original[..2].to_vec();
        oriented.extend_from_slice(&[0xFF, 0xE1]);
        oriented.extend_from_slice(&u16::try_from(exif.len() + 2).unwrap().to_be_bytes());
        oriented.append(&mut exif);
        oriented.extend_from_slice(&original[2..]);
        let page = JpegPage::read(
            &format!("data:image/jpeg;base64,{}", STANDARD.encode(&oriented)),
            "White - front",
        )
        .unwrap();
        assert_eq!(page.orientation, Orientation::Rotate90);
        let (w, h, matrix) = placement(&page);
        assert!(w < h);
        assert_eq!(matrix[0], 0.0);
        assert!(matrix[1] < 0.0 && matrix[2] > 0.0);
        assert_eq!(page.bytes, oriented);
    }

    #[test]
    fn application_rotations_reach_pdf_matrices_and_exact_embedded_streams() {
        use passport_scanner_core::{Frame, LightSource, PixelFormat};
        for (angle, expected) in [
            (0, Orientation::NoTransforms),
            (90, Orientation::Rotate90),
            (180, Orientation::Rotate180),
            (-90, Orientation::Rotate270),
        ] {
            let mut frames = [Frame {
                width: 12,
                height: 8,
                format: PixelFormat::Jpeg,
                source: LightSource::White,
                data: jpeg([10, 60, 120], 12, 8),
            }];
            crate::orientation::orient_frames(&mut frames, angle).unwrap();
            let page = JpegPage::read(
                &format!("data:image/jpeg;base64,{}", STANDARD.encode(&frames[0].data)),
                "White - front",
            )
            .unwrap();
            assert_eq!(page.orientation, expected);
            let (_, _, matrix) = placement(&page);
            match angle {
                0 => assert!(matrix[0] > 0.0 && matrix[3] > 0.0),
                90 => assert!(matrix[0] == 0.0 && matrix[1] < 0.0 && matrix[2] > 0.0),
                180 => assert!(matrix[0] < 0.0 && matrix[3] < 0.0),
                -90 => assert!(matrix[0] == 0.0 && matrix[1] > 0.0 && matrix[2] < 0.0),
                _ => unreachable!(),
            }
            let pdf = build_pdf(&[page]).unwrap();
            assert!(pdf.windows(frames[0].data.len()).any(|bytes| bytes == frames[0].data));
            assert!(String::from_utf8_lossy(&pdf).contains("/Width 12 /Height 8"));
        }
    }
}
