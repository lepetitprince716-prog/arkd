use arkd_core::hud::HudTemplate;
use arkd_core::session::BgrFrame;

fn rgb_to_png(img: &image::RgbImage) -> Vec<u8> {
    let mut out = Vec::new();
    image::DynamicImage::ImageRgb8(img.clone())
        .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
        .unwrap();
    out
}

fn rgb_to_bgr(img: &image::RgbImage) -> Vec<u8> {
    let mut bgr = Vec::with_capacity(img.width() as usize * img.height() as usize * 3);
    for p in img.pixels() {
        bgr.push(p[2]);
        bgr.push(p[1]);
        bgr.push(p[0]);
    }
    bgr
}

fn patterned_image(w: u32, h: u32, seed: u8) -> image::RgbImage {
    let mut img = image::RgbImage::new(w, h);
    for (x, y, p) in img.enumerate_pixels_mut() {
        let v =
            (((x / 8) * 31 + (y / 8) * 17 + seed as u32 * 97).wrapping_mul(2654435761) % 251) as u8;
        *p = image::Rgb([v, (v / 2).wrapping_add(x as u8), 255u8.wrapping_sub(v)]);
    }
    img
}

fn frame_of(img: &image::RgbImage) -> BgrFrame {
    BgrFrame {
        width: img.width(),
        height: img.height(),
        data: rgb_to_bgr(img),
    }
}

#[test]
fn template_pasted_at_roi_scores_high() {
    let src = patterned_image(1280, 720, 7);
    let template = HudTemplate::from_png(&rgb_to_png(&src), (1180, 30, 60, 50), 0.85).unwrap();
    let frame = frame_of(&src);
    let score = template.score(&frame).unwrap();
    assert!(score > 0.95, "score {score}");
    assert!(template.matches(&frame).unwrap());
}

#[test]
fn unrelated_noise_scores_low() {
    let src = patterned_image(1280, 720, 7);
    let template = HudTemplate::from_png(&rgb_to_png(&src), (1180, 30, 60, 50), 0.85).unwrap();
    let noise = patterned_image(1280, 720, 200);
    let frame = frame_of(&noise);
    let score = template.score(&frame).unwrap();
    assert!(score < 0.3, "score {score}");
    assert!(!template.matches(&frame).unwrap());
}

#[test]
fn scaled_frame_scores_through_roi_scaling() {
    let src = patterned_image(1280, 720, 11);
    let template = HudTemplate::from_png(&rgb_to_png(&src), (1180, 30, 60, 50), 0.85).unwrap();
    let small = image::imageops::resize(&src, 640, 360, image::imageops::FilterType::Triangle);
    let frame = frame_of(&small);
    let score = template.score(&frame).unwrap();
    assert!(score > 0.8, "score {score}");
}
