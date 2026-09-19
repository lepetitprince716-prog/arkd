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
    let template =
        HudTemplate::from_png(&rgb_to_png(&src), (1180, 30, 60, 50), (1280, 720), 0.85).unwrap();
    let frame = frame_of(&src);
    let score = template.score(&frame).unwrap();
    assert!(score > 0.95, "score {score}");
    assert!(template.matches(&frame).unwrap());
}

#[test]
fn unrelated_noise_scores_low() {
    let src = patterned_image(1280, 720, 7);
    let template =
        HudTemplate::from_png(&rgb_to_png(&src), (1180, 30, 60, 50), (1280, 720), 0.85).unwrap();
    let noise = patterned_image(1280, 720, 200);
    let frame = frame_of(&noise);
    let score = template.score(&frame).unwrap();
    assert!(score < 0.3, "score {score}");
    assert!(!template.matches(&frame).unwrap());
}

const RUNNING_TPL: &[u8] = include_bytes!("../assets/hud/playcover_running.png");
const PAUSED_TPL: &[u8] = include_bytes!("../assets/hud/playcover_paused.png");
const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/hud/fixtures");
const ROI: (u32, u32, u32, u32) = (1180, 30, 60, 50);
const SPACE: (u32, u32) = (1280, 720);

fn frame_with_crop(crop_png: &[u8]) -> BgrFrame {
    let crop = image::load_from_memory(crop_png).unwrap().to_rgb8();
    let mut img = image::RgbImage::new(SPACE.0, SPACE.1);
    image::imageops::overlay(&mut img, &crop, ROI.0 as i64, ROI.1 as i64);
    frame_of(&img)
}

fn fixture(name: &str) -> BgrFrame {
    frame_with_crop(&std::fs::read(format!("{FIXTURES}/{name}")).unwrap())
}

#[test]
fn embedded_templates_score_live_fixtures() {
    let running_tpl = HudTemplate::from_png(RUNNING_TPL, ROI, SPACE, 0.85).unwrap();
    let paused_tpl = HudTemplate::from_png(PAUSED_TPL, ROI, SPACE, 0.85).unwrap();

    let running = fixture("running_b_roi.png");
    let formation = fixture("formation_roi.png");
    let paused = frame_with_crop(PAUSED_TPL);
    let running_self = frame_with_crop(RUNNING_TPL);

    let r_self = running_tpl.score(&running_self).unwrap();
    let r_on_running = running_tpl.score(&running).unwrap();
    let r_on_formation = running_tpl.score(&formation).unwrap();
    let r_on_paused = running_tpl.score(&paused).unwrap();
    let p_on_paused = paused_tpl.score(&paused).unwrap();
    let p_on_running = paused_tpl.score(&running_self).unwrap();
    eprintln!(
        "hud scores: run->run={r_self:.4} run->running_b={r_on_running:.4} run->formation={r_on_formation:.4} run->paused={r_on_paused:.4} paused->paused={p_on_paused:.4} paused->running={p_on_running:.4}"
    );
    assert!(r_self >= 0.99, "running self score {r_self}");
    assert!(r_on_running >= 0.95, "running_b score {r_on_running}");
    assert!(r_on_formation < 0.5, "formation score {r_on_formation}");
    assert!(r_on_paused < 0.85, "paused-vs-running score {r_on_paused}");
    assert!(p_on_paused >= 0.99, "paused self score {p_on_paused}");
    assert!(
        p_on_running < 0.85,
        "running-vs-paused score {p_on_running}"
    );
    assert!(running_tpl.matches(&running).unwrap());
    assert!(!running_tpl.matches(&paused).unwrap());
    assert!(paused_tpl.matches(&paused).unwrap());
    assert!(!paused_tpl.matches(&running).unwrap());
}

#[test]
fn a_png_sized_like_the_roi_is_taken_as_the_crop() {
    let src = patterned_image(1280, 720, 7);
    let crop = image::imageops::crop_imm(&src, 1180, 30, 60, 50).to_image();
    let template =
        HudTemplate::from_png(&rgb_to_png(&crop), (1180, 30, 60, 50), (1280, 720), 0.85).unwrap();
    let frame = frame_of(&src);
    let score = template.score(&frame).unwrap();
    assert!(score > 0.99, "score {score}");
    assert!(template.matches(&frame).unwrap());
}

#[test]
fn scaled_frame_scores_through_roi_scaling() {
    let src = patterned_image(1280, 720, 11);
    let template =
        HudTemplate::from_png(&rgb_to_png(&src), (1180, 30, 60, 50), (1280, 720), 0.85).unwrap();
    let small = image::imageops::resize(&src, 640, 360, image::imageops::FilterType::Triangle);
    let frame = frame_of(&small);
    let score = template.score(&frame).unwrap();
    assert!(score > 0.8, "score {score}");
}
