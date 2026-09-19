#![cfg(feature = "fake")]

use arkd_core::playtools::Frame;
use arkd_core::screen::{
    CoordSpace, EncodeOpts, Geometry, ImageFormat, encode_frame, encode_frame_normalised,
    encode_png, png_dimensions,
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
fn maacore_click_space_is_normalised() {
    let geo = Geometry {
        device: (1920, 1080),
        screenshot: (1280, 720),
    };
    // 16:9-or-wider device: MaaCore's click frame is 1280x720
    assert_eq!(
        geo.to_maacore(1210.0, 55.0, CoordSpace::Screenshot),
        (1210, 55)
    );
    assert_eq!(geo.to_maacore(1815.0, 82.0, CoordSpace::Device), (1210, 55));

    let geo = Geometry {
        device: (2560, 1440),
        screenshot: (1280, 720),
    };
    assert_eq!(
        geo.to_maacore(640.0, 360.0, CoordSpace::Screenshot),
        (640, 360)
    );

    // narrower than 16:9: MaaCore scales on width (1280/dw) instead
    let geo = Geometry {
        device: (1600, 1200),
        screenshot: (1280, 720),
    };
    // screenshot (640, 360) -> device (800, 600) -> core (640, 480)
    assert_eq!(
        geo.to_maacore(640.0, 360.0, CoordSpace::Screenshot),
        (640, 480)
    );
    // device (800, 600) -> core (640, 480)
    assert_eq!(geo.to_maacore(800.0, 600.0, CoordSpace::Device), (640, 480));
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
fn encode_frame_to_jpeg_works_on_bgr_frames() {
    let frame = Frame {
        width: 8,
        height: 4,
        bgr: vec![10u8; 8 * 4 * 3],
    };
    let enc = encode_frame(
        &frame,
        EncodeOpts {
            scale: 1.0,
            format: ImageFormat::Jpeg,
            quality: 80,
        },
    )
    .unwrap();
    assert_eq!(enc.mime, "image/jpeg");
    assert_eq!(&enc.bytes[..2], &[0xFF, 0xD8]);
    assert_eq!((enc.width, enc.height), (8, 4));
}

#[test]
fn playtools_frame_is_resized_to_screenshot_space() {
    let frame = Frame {
        width: 296,
        height: 166,
        bgr: vec![42u8; 296 * 166 * 3],
    };
    let enc = encode_frame_normalised(&frame, (128, 72), EncodeOpts::default()).unwrap();
    assert_eq!((enc.width, enc.height), (128, 72));
    assert_eq!(png_dimensions(&enc.bytes).unwrap(), (128, 72));

    let enc = encode_frame_normalised(
        &frame,
        (128, 72),
        EncodeOpts {
            scale: 0.5,
            format: ImageFormat::Png,
            quality: 80,
        },
    )
    .unwrap();
    assert_eq!((enc.width, enc.height), (64, 36));
    assert_eq!(png_dimensions(&enc.bytes).unwrap(), (64, 36));
}

#[test]
fn playtools_frame_at_target_size_is_not_resampled() {
    let mut bgr = Vec::with_capacity(128 * 72 * 3);
    let mut rgb = Vec::with_capacity(128 * 72 * 3);
    for i in 0..(128 * 72) {
        let (b, g, r) = (
            (i % 251) as u8,
            ((i * 3) % 253) as u8,
            ((i * 7) % 255) as u8,
        );
        bgr.extend_from_slice(&[b, g, r]);
        rgb.extend_from_slice(&[r, g, b]);
    }
    let frame = Frame {
        width: 128,
        height: 72,
        bgr,
    };
    let enc = encode_frame_normalised(&frame, (128, 72), EncodeOpts::default()).unwrap();
    let decoded = image::load_from_memory(&enc.bytes).unwrap().to_rgb8();
    assert_eq!(decoded.as_raw(), &rgb);
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
