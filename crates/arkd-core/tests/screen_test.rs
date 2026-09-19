#![cfg(feature = "fake")]

use arkd_core::screen::{
    CoordSpace, EncodeOpts, Geometry, ImageFormat, encode_png, png_dimensions,
};

fn test_png(w: u32, h: u32) -> Vec<u8> {
    let img = image::RgbaImage::from_pixel(w, h, image::Rgba([10, 20, 30, 255]));
    let mut out = Vec::new();
    image::ImageEncoder::write_image(
        image::codecs::png::PngEncoder::new(&mut out),
        img.as_raw(),
        w,
        h,
        image::ExtendedColorType::Rgba8,
    )
    .unwrap();
    out
}

#[test]
fn screenshot_to_device_mapping() {
    let geo = Geometry {
        device: (1920, 1080),
        screenshot: (1280, 720),
    };
    assert_eq!(
        geo.to_device(1210.0, 55.0, CoordSpace::Screenshot),
        (1815, 82)
    );
    assert_eq!(
        geo.to_screenshot(1815.0, 82.0, CoordSpace::Device),
        (1210, 55)
    );
}

#[test]
fn encode_passthrough_when_no_scaling() {
    let png = test_png(100, 50);
    let enc = encode_png(&png, EncodeOpts::default()).unwrap();
    assert_eq!(enc.mime, "image/png");
    assert_eq!((enc.width, enc.height), (100, 50));
    assert_eq!(enc.bytes, png);
}

#[test]
fn scale_half_halves_dimensions() {
    let png = test_png(100, 50);
    let enc = encode_png(
        &png,
        EncodeOpts {
            scale: 0.5,
            format: ImageFormat::Png,
            quality: 80,
        },
    )
    .unwrap();
    assert_eq!((enc.width, enc.height), (50, 25));
    assert_eq!(png_dimensions(&enc.bytes).unwrap(), (50, 25));
}

#[test]
fn jpeg_mime_and_magic() {
    let png = test_png(32, 32);
    let enc = encode_png(
        &png,
        EncodeOpts {
            scale: 1.0,
            format: ImageFormat::Jpeg,
            quality: 80,
        },
    )
    .unwrap();
    assert_eq!(enc.mime, "image/jpeg");
    assert_eq!(&enc.bytes[..2], &[0xFF, 0xD8]);
}

#[test]
fn events_ring_buffer_drop_count() {
    let mut log = arkd_core::events::EventLog::new(3);
    for i in 0..10 {
        log.push(10002, serde_json::json!({"taskid": i}));
    }
    let page = log.page(1, 50, false, false);
    assert!(page.dropped > 0);
    assert_eq!(page.latest_seq, 10);
}

#[test]
fn describe_carries_details_for_battle_events_only() {
    let mut log = arkd_core::events::EventLog::new(10);
    log.push(
        20003,
        serde_json::json!({"what": "BattleFormation", "details": {"formation": ["A"]}}),
    );
    log.push(
        20003,
        serde_json::json!({"what": "StageDrops", "details": {"x": 1}}),
    );
    let page = log.page(0, 10, false, false);
    assert!(page.events[0].get("details").is_some());
    assert!(page.events[1].get("details").is_none());
}
