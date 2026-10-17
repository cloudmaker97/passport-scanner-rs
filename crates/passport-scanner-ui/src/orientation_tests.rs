//! Verify the real Iced renderer path; no parallel production rotation code.
use super::*;

fn fixture() -> Vec<u8> {
    let pixels = ::image::RgbImage::from_fn(48, 32, |x, y| {
        ::image::Rgb(match (x < 24, y < 16) {
            (true, true) => [230, 20, 30],
            (false, true) => [30, 220, 40],
            (false, false) => [40, 30, 230],
            (true, false) => [220, 200, 20],
        })
    });
    let mut bytes = Vec::new();
    ::image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, 100)
        .encode_image(&pixels)
        .unwrap();
    bytes
}

/// Independent minimal EXIF fixture; preserves the original JPEG codestream.
fn tagged(jpeg: &[u8], orientation: u16) -> Vec<u8> {
    let mut exif = b"Exif\0\0II*\0\x08\0\0\0\x01\0\x12\x01\x03\0\x01\0\0\0".to_vec();
    exif.extend(orientation.to_le_bytes());
    exif.extend([0; 6]);
    let mut result = jpeg[..2].to_vec();
    result.extend([0xff, 0xe1]);
    result.extend(((exif.len() + 2) as u16).to_be_bytes());
    result.extend(exif);
    result.extend(&jpeg[2..]);
    result
}

fn corners(image: &iced_graphics::image::Buffer) -> [[u8; 4]; 4] {
    [
        (2, 2),
        (image.width() - 3, 2),
        (image.width() - 3, image.height() - 3),
        (2, image.height() - 3),
    ]
    .map(|(x, y)| image.get_pixel(x, y).0)
}

#[test]
fn real_iced_loader_orients_all_four_angles_without_changing_jpeg_bytes() {
    let jpeg = fixture();
    let original = iced_graphics::image::load(&image::Handle::from_bytes(jpeg.clone())).unwrap();
    let reference = corners(&original);
    assert!(reference.windows(2).all(|pair| pair[0] != pair[1]));
    for (tag, dimensions, order) in [
        (1, (48, 32), [0, 1, 2, 3]),
        (6, (32, 48), [3, 0, 1, 2]),
        (3, (48, 32), [2, 3, 0, 1]),
        (8, (32, 48), [1, 2, 3, 0]),
    ] {
        let oriented = tagged(&jpeg, tag);
        assert_eq!(&oriented[38..], &jpeg[2..], "EXIF only; no JPEG re-encoding");
        let handle = image::Handle::from_bytes(oriented.clone());
        let loaded = iced_graphics::image::load(&handle).unwrap();
        assert_eq!(loaded.dimensions(), dimensions, "orientation {tag}");
        assert_eq!(corners(&loaded), order.map(|index| reference[index]), "orientation {tag}");
        match handle {
            image::Handle::Bytes(_, bytes) => assert_eq!(&*bytes, oriented.as_slice()),
            _ => panic!("image must stay encoded until renderer load"),
        }
    }
}

#[test]
fn orientation_tag_replacement_refreshes_only_changed_handle_and_survives_history() {
    let jpeg = fixture();
    let mut scan = Scan {
        id: "oriented".into(),
        document: "one".into(),
        side: Side::Back,
        at: String::new(),
        format: None,
        mrz: vec![],
        mrz_check: None,
        fields: vec![],
        images: vec![passport_scanner_protocol::Image {
            source: Source::White,
            url: format!(
                "data:image/jpeg;base64,{}",
                base64::engine::general_purpose::STANDARD.encode(tagged(&jpeg, 6))
            ),
            width: 32,
            height: 48,
        }],
    };
    let mut desktop = App::unconnected(Box::new(Offline), Box::<Memory>::default());
    desktop.receive(Event::ScanCompleted { scan: Box::new(scan.clone()) });
    let initial = desktop.images["oriented:0"].id();
    desktop.receive(Event::State {
        device: None,
        scans: vec![scan.clone()],
        selected: Some(scan.id.clone()),
    });
    assert_eq!(desktop.images["oriented:0"].id(), initial);
    assert_eq!(
        iced_graphics::image::load(&desktop.images["oriented:0"]).unwrap().dimensions(),
        (32, 48)
    );
    scan.images[0].url = format!(
        "data:image/jpeg;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(tagged(&jpeg, 3))
    );
    scan.images[0].width = 48;
    scan.images[0].height = 32;
    desktop.receive(Event::State {
        device: None,
        scans: vec![scan.clone()],
        selected: Some(scan.id),
    });
    assert_ne!(desktop.images["oriented:0"].id(), initial);
    assert_eq!(
        iced_graphics::image::load(&desktop.images["oriented:0"]).unwrap().dimensions(),
        (48, 32)
    );
    assert_eq!(
        desktop.selected_document().unwrap().image(Side::Back, Source::White).unwrap().width,
        48
    );
}
