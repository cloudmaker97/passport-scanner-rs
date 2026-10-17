//! Lossless display rotation: only EXIF metadata changes, never JPEG coefficients.
//! Frame dimensions continue to describe the encoded raster. Use
//! [`display_dimensions`] when exposing dimensions to a viewer.

use image::{ImageDecoder, codecs::jpeg::JpegDecoder};
use passport_scanner_core::{Error, Frame, PixelFormat, Result};
use std::{io::Cursor, ops::Range};

/// Rotate the currently displayed image clockwise by 0, 90, 180, or -90 degrees.
/// All frames are validated before any is changed. Existing EXIF transforms,
/// including mirrored orientations, are composed with the requested rotation.
pub fn orient_frames(frames: &mut [Frame], angle: i16) -> Result<()> {
    let turns = match angle {
        0 => 0,
        90 => 1,
        180 => 2,
        -90 => 3,
        _ => {
            return Err(Error::Unsupported(
                "Orientation must be 0, 90, 180, or -90 degrees".into(),
            ));
        }
    };
    let updates = frames
        .iter()
        .map(|frame| {
            require_jpeg(frame)?;
            let info = inspect(&frame.data)?;
            if turns == 0 {
                return Ok(None);
            }
            let orientation = compose(info.orientation, turns);
            let mut bytes = frame.data.clone();
            if let Some(range) = info.exif {
                let mut payload = bytes[range.start + 4..range.end].to_vec();
                set_exif_orientation(&mut payload, orientation)?;
                let length = u16::try_from(payload.len() + 2).map_err(|_| {
                    Error::Unsupported("EXIF is too large to add orientation".into())
                })?;
                let mut segment = vec![0xff, 0xe1];
                segment.extend_from_slice(&length.to_be_bytes());
                segment.extend_from_slice(&payload);
                bytes.splice(range, segment);
            } else {
                let payload = minimal_exif(orientation);
                let mut segment = vec![0xff, 0xe1];
                segment.extend_from_slice(&((payload.len() + 2) as u16).to_be_bytes());
                segment.extend_from_slice(&payload);
                bytes.splice(2..2, segment);
            }
            Ok(Some(bytes))
        })
        .collect::<Result<Vec<_>>>()?;
    for (frame, update) in frames.iter_mut().zip(updates) {
        if let Some(bytes) = update {
            frame.data = bytes;
        }
    }
    Ok(())
}

/// Dimensions after applying EXIF orientation, derived from the JPEG itself.
pub fn display_dimensions(frame: &Frame) -> Result<(u32, u32)> {
    require_jpeg(frame)?;
    let info = inspect(&frame.data)?;
    let (width, height) = info.dimensions;
    Ok(if info.orientation >= 5 { (height, width) } else { (width, height) })
}

fn require_jpeg(frame: &Frame) -> Result<()> {
    if frame.format != PixelFormat::Jpeg {
        return Err(Error::Unsupported("Lossless display orientation requires JPEG frames".into()));
    }
    Ok(())
}

fn malformed() -> Error {
    Error::Malformed("Invalid JPEG or EXIF orientation metadata".into())
}

struct Info {
    exif: Option<Range<usize>>,
    orientation: u16,
    dimensions: (u32, u32),
}

// Validate segment boundaries and scan termination without decoding/re-encoding
// pixels. Entropy bytes (including stuffed FF and restart markers) stay untouched.
fn inspect(bytes: &[u8]) -> Result<Info> {
    if !bytes.starts_with(&[0xff, 0xd8]) {
        return Err(malformed());
    }
    let mut at = 2;
    let mut exif = None;
    let mut orientation = 1;
    let mut in_scan = false;
    let mut saw_scan = false;
    loop {
        if in_scan {
            while bytes.get(at).is_some_and(|byte| *byte != 0xff) {
                at += 1;
            }
        }
        if bytes.get(at) != Some(&0xff) {
            return Err(malformed());
        }
        while bytes.get(at) == Some(&0xff) {
            at += 1;
        }
        let marker = *bytes.get(at).ok_or_else(malformed)?;
        let start = at - 1;
        at += 1;
        if in_scan && (marker == 0 || (0xd0..=0xd7).contains(&marker)) {
            continue;
        }
        in_scan = false;
        if marker == 0xd9 {
            if !saw_scan || at != bytes.len() {
                return Err(malformed());
            }
            break;
        }
        if marker == 0 || (0xd0..=0xd8).contains(&marker) {
            return Err(malformed());
        }
        if marker == 1 {
            continue;
        }
        let length = bytes.get(at..at + 2).ok_or_else(malformed)?;
        let length = usize::from(u16::from_be_bytes([length[0], length[1]]));
        if length < 2 {
            return Err(malformed());
        }
        let payload = bytes.get(at + 2..at + length).ok_or_else(malformed)?;
        if marker == 0xe1 && payload.starts_with(b"Exif\0\0") {
            if exif.is_some() || saw_scan {
                return Err(Error::Unsupported("Ambiguous JPEG EXIF segments".into()));
            }
            let parsed = parse_exif(payload)?;
            orientation = parsed.orientation;
            exif = Some(start..at + length);
        }
        at += length;
        if marker == 0xda {
            saw_scan = true;
            in_scan = true;
        }
    }
    let decoder = JpegDecoder::new(Cursor::new(bytes)).map_err(Error::backend)?;
    let dimensions = decoder.dimensions();
    if dimensions.0 == 0 || dimensions.1 == 0 {
        return Err(malformed());
    }
    Ok(Info { exif, orientation, dimensions })
}

struct Exif {
    little: bool,
    ifd: usize,
    count: usize,
    value_offset: Option<usize>,
    orientation: u16,
}

fn read16(bytes: &[u8], at: usize, little: bool) -> Result<u16> {
    let value: [u8; 2] = bytes.get(at..at + 2).ok_or_else(malformed)?.try_into().unwrap();
    Ok(if little { u16::from_le_bytes(value) } else { u16::from_be_bytes(value) })
}

fn read32(bytes: &[u8], at: usize, little: bool) -> Result<u32> {
    let value: [u8; 4] = bytes.get(at..at + 4).ok_or_else(malformed)?.try_into().unwrap();
    Ok(if little { u32::from_le_bytes(value) } else { u32::from_be_bytes(value) })
}

fn encoded16(value: u16, little: bool) -> [u8; 2] {
    if little { value.to_le_bytes() } else { value.to_be_bytes() }
}

fn encoded32(value: u32, little: bool) -> [u8; 4] {
    if little { value.to_le_bytes() } else { value.to_be_bytes() }
}

fn parse_exif(payload: &[u8]) -> Result<Exif> {
    let tiff = payload.get(6..).ok_or_else(malformed)?;
    let little = match tiff.get(..2) {
        Some(b"II") => true,
        Some(b"MM") => false,
        _ => return Err(malformed()),
    };
    if read16(tiff, 2, little)? != 42 {
        return Err(malformed());
    }
    let ifd = usize::try_from(read32(tiff, 4, little)?).map_err(Error::backend)?;
    if ifd < 8 {
        return Err(malformed());
    }
    let count = usize::from(read16(tiff, ifd, little)?);
    tiff.get(ifd + 2..ifd + 2 + count * 12 + 4).ok_or_else(malformed)?;
    let mut value_offset = None;
    let mut orientation = 1;
    for n in 0..count {
        let entry = ifd + 2 + n * 12;
        if read16(tiff, entry, little)? == 0x0112 {
            if value_offset.is_some()
                || read16(tiff, entry + 2, little)? != 3
                || read32(tiff, entry + 4, little)? != 1
            {
                return Err(malformed());
            }
            orientation = read16(tiff, entry + 8, little)?;
            if !(1..=8).contains(&orientation) {
                return Err(malformed());
            }
            value_offset = Some(6 + entry + 8);
        }
    }
    Ok(Exif { little, ifd, count, value_offset, orientation })
}

fn set_exif_orientation(payload: &mut Vec<u8>, orientation: u16) -> Result<()> {
    let exif = parse_exif(payload)?;
    if let Some(at) = exif.value_offset {
        payload[at..at + 2].copy_from_slice(&encoded16(orientation, exif.little));
        return Ok(());
    }
    // Append a replacement IFD0. Existing metadata and all TIFF-relative offsets
    // remain valid; only the root IFD pointer changes. Keep the thumbnail chain.
    let tiff = &payload[6..];
    let mut entries = (0..exif.count)
        .map(|n| tiff[exif.ifd + 2 + n * 12..exif.ifd + 14 + n * 12].to_vec())
        .collect::<Vec<_>>();
    let mut entry = Vec::new();
    entry.extend_from_slice(&encoded16(0x0112, exif.little));
    entry.extend_from_slice(&encoded16(3, exif.little));
    entry.extend_from_slice(&encoded32(1, exif.little));
    entry.extend_from_slice(&encoded16(orientation, exif.little));
    entry.extend_from_slice(&[0, 0]);
    entries.push(entry);
    entries.sort_by_key(|entry| read16(entry, 0, exif.little).unwrap());
    let next = tiff[exif.ifd + 2 + exif.count * 12..exif.ifd + 6 + exif.count * 12].to_vec();
    let new_ifd = tiff.len() + tiff.len() % 2;
    if tiff.len() % 2 != 0 {
        payload.push(0);
    }
    payload[10..14].copy_from_slice(&encoded32(new_ifd as u32, exif.little));
    payload.extend_from_slice(&encoded16(entries.len() as u16, exif.little));
    for entry in entries {
        payload.extend_from_slice(&entry);
    }
    payload.extend_from_slice(&next);
    Ok(())
}

fn minimal_exif(orientation: u16) -> Vec<u8> {
    let mut payload = b"Exif\0\0II\x2a\0\x08\0\0\0\x01\0\x12\x01\x03\0\x01\0\0\0".to_vec();
    payload.extend_from_slice(&orientation.to_le_bytes());
    payload.extend_from_slice(&[0; 6]);
    payload
}

fn compose(mut orientation: u16, turns: u8) -> u16 {
    // Apply a clockwise quarter-turn AFTER the existing EXIF transform.
    const CLOCKWISE: [u16; 8] = [6, 7, 8, 5, 2, 3, 4, 1];
    for _ in 0..turns {
        orientation = CLOCKWISE[usize::from(orientation - 1)];
    }
    orientation
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{DynamicImage, metadata::Orientation};
    use passport_scanner_core::LightSource;

    fn frame() -> Frame {
        let pixels = image::RgbImage::from_fn(17, 11, |x, y| {
            image::Rgb([(x * 13) as u8, (y * 21) as u8, ((x + y) * 7) as u8])
        });
        let mut data = Vec::new();
        image::codecs::jpeg::JpegEncoder::new(&mut data).encode_image(&pixels).unwrap();
        Frame { width: 17, height: 11, format: PixelFormat::Jpeg, source: LightSource::White, data }
    }

    fn with_exif(mut frame: Frame, payload: &[u8]) -> Frame {
        let mut segment = vec![0xff, 0xe1];
        segment.extend_from_slice(&((payload.len() + 2) as u16).to_be_bytes());
        segment.extend_from_slice(payload);
        frame.data.splice(2..2, segment);
        frame
    }

    fn pixels(frame: &Frame, apply: bool) -> DynamicImage {
        let mut image = image::load_from_memory(&frame.data).unwrap();
        if apply {
            let mut decoder = JpegDecoder::new(Cursor::new(&frame.data)).unwrap();
            image.apply_orientation(decoder.orientation().unwrap());
        }
        image
    }

    fn without_exif(frame: &Frame) -> Vec<u8> {
        let mut bytes = frame.data.clone();
        if let Some(range) = inspect(&bytes).unwrap().exif {
            bytes.drain(range);
        }
        bytes
    }

    #[test]
    fn all_angles_preserve_exact_jpeg_data_and_encoded_dimensions() {
        let original = frame();
        let original_pixels = pixels(&original, false);
        for (angle, orientation, dimensions) in
            [(0, 1, (17, 11)), (90, 6, (11, 17)), (180, 3, (17, 11)), (-90, 8, (11, 17))]
        {
            let mut frames = [original.clone()];
            orient_frames(&mut frames, angle).unwrap();
            assert_eq!(inspect(&frames[0].data).unwrap().orientation, orientation);
            assert_eq!(without_exif(&frames[0]), original.data);
            assert_eq!(pixels(&frames[0], false).to_rgb8(), original_pixels.to_rgb8());
            assert_eq!((frames[0].width, frames[0].height), (17, 11));
            assert_eq!(display_dimensions(&frames[0]).unwrap(), dimensions);
            if angle == 0 {
                assert_eq!(frames[0].data, original.data);
            }
        }
    }

    #[test]
    fn all_existing_exif_transforms_compose_in_display_order() {
        for existing in 1..=8 {
            let original = with_exif(frame(), &minimal_exif(existing));
            let displayed = pixels(&original, true);
            for angle in [0, 90, 180, -90] {
                let expected = match angle {
                    90 => displayed.rotate90(),
                    180 => displayed.rotate180(),
                    -90 => displayed.rotate270(),
                    _ => displayed.clone(),
                };
                let mut frames = [original.clone()];
                orient_frames(&mut frames, angle).unwrap();
                assert_eq!(
                    pixels(&frames[0], true).to_rgb8(),
                    expected.to_rgb8(),
                    "EXIF {existing}, angle {angle}"
                );
                assert_eq!(without_exif(&frames[0]), without_exif(&original));
                let changed =
                    frames[0].data.iter().zip(&original.data).filter(|(a, b)| a != b).count();
                assert!(changed <= 1, "only the orientation value changes");
            }
        }
    }

    #[test]
    fn big_endian_exif_and_unrelated_metadata_are_preserved() {
        let payload = b"Exif\0\0MM\0\x2a\0\0\0\x08\0\x01\x01\x12\0\x03\0\0\0\x01\0\x08\0\0\0\0\0\0unrelated metadata";
        let original = with_exif(frame(), payload);
        let mut frames = [original.clone()];
        orient_frames(&mut frames, 90).unwrap();
        assert_eq!(inspect(&frames[0].data).unwrap().orientation, 1);
        assert_eq!(frames[0].data.len(), original.data.len());
        assert_eq!(frames[0].data.iter().zip(&original.data).filter(|(a, b)| a != b).count(), 1);
        assert_eq!(
            pixels(&frames[0], true).to_rgb8(),
            pixels(&original, true).rotate90().to_rgb8()
        );
    }

    #[test]
    fn exif_without_orientation_keeps_metadata_when_adding_root_entry() {
        let payload = b"Exif\0\0II\x2a\0\x08\0\0\0\0\0\0\0\0\0unrelated metadata";
        let original = with_exif(frame(), payload);
        let mut frames = [original.clone()];
        orient_frames(&mut frames, -90).unwrap();
        let info = inspect(&frames[0].data).unwrap();
        assert_eq!(info.orientation, 8);
        let range = info.exif.unwrap();
        let updated = &frames[0].data[range.start + 4..range.end];
        assert_eq!(&updated[14..payload.len()], &payload[14..]);
        assert_eq!(without_exif(&frames[0]), without_exif(&original));
        let mut decoder = JpegDecoder::new(Cursor::new(&frames[0].data)).unwrap();
        assert_eq!(decoder.orientation().unwrap(), Orientation::Rotate270);
    }

    #[test]
    fn invalid_inputs_leave_every_frame_unchanged() {
        let original = frame();
        let mut malformed_frames = vec![original.clone(); 4];
        malformed_frames[0].data.pop(); // missing EOI
        malformed_frames[1].data = vec![0xff, 0xd8, 0xff, 0xe1, 0xff, 0xff, 0xff, 0xd9];
        malformed_frames[2] = with_exif(frame(), &minimal_exif(9));
        malformed_frames[3] = with_exif(with_exif(frame(), &minimal_exif(1)), &minimal_exif(1));
        for bad in malformed_frames {
            let mut frames = [original.clone(), bad.clone()];
            assert!(orient_frames(&mut frames, 90).is_err());
            assert_eq!(frames[0].data, original.data);
            assert_eq!(frames[1].data, bad.data);
        }
        let mut frames = [original.clone()];
        assert!(orient_frames(&mut frames, 45).is_err());
        assert_eq!(frames[0].data, original.data);
    }
}
