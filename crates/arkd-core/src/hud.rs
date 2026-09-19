use std::path::Path;

use crate::error::{Error, Result};
use crate::session::BgrFrame;

pub struct HudTemplate {
    roi: (u32, u32, u32, u32),
    src: (u32, u32),
    gray: Vec<f32>,
    w: u32,
    h: u32,
    threshold: f64,
}

fn crop_gray_bgr(data: &[u8], w: u32, roi: (u32, u32, u32, u32)) -> Vec<f32> {
    let (x, y, rw, rh) = roi;
    let mut gray = Vec::with_capacity((rw * rh) as usize);
    for j in 0..rh {
        for i in 0..rw {
            let px = (((y + j) * w + x + i) as usize) * 3;
            let b = data[px] as f32;
            let g = data[px + 1] as f32;
            let r = data[px + 2] as f32;
            gray.push(0.114 * b + 0.587 * g + 0.299 * r);
        }
    }
    gray
}

fn resize_bilinear(gray: &[f32], w: u32, h: u32, tw: u32, th: u32) -> Vec<f32> {
    let mut out = Vec::with_capacity((tw * th) as usize);
    for j in 0..th {
        let sy = if th > 1 {
            j as f32 * (h - 1) as f32 / (th - 1) as f32
        } else {
            0.0
        };
        for i in 0..tw {
            let sx = if tw > 1 {
                i as f32 * (w - 1) as f32 / (tw - 1) as f32
            } else {
                0.0
            };
            let (x0, y0) = (sx.floor() as u32, sy.floor() as u32);
            let (x1, y1) = ((x0 + 1).min(w - 1), (y0 + 1).min(h - 1));
            let (fx, fy) = (sx - x0 as f32, sy - y0 as f32);
            let v00 = gray[(y0 * w + x0) as usize];
            let v10 = gray[(y0 * w + x1) as usize];
            let v01 = gray[(y1 * w + x0) as usize];
            let v11 = gray[(y1 * w + x1) as usize];
            out.push(
                v00 * (1.0 - fx) * (1.0 - fy)
                    + v10 * fx * (1.0 - fy)
                    + v01 * (1.0 - fx) * fy
                    + v11 * fx * fy,
            );
        }
    }
    out
}

fn ncc(template: &[f32], patch: &[f32]) -> f64 {
    let n = template.len() as f64;
    let mean_t = template.iter().map(|v| *v as f64).sum::<f64>() / n;
    let mean_p = patch.iter().map(|v| *v as f64).sum::<f64>() / n;
    let (mut num, mut dt, mut dp) = (0.0f64, 0.0f64, 0.0f64);
    for (t, p) in template.iter().zip(patch.iter()) {
        let a = *t as f64 - mean_t;
        let b = *p as f64 - mean_p;
        num += a * b;
        dt += a * a;
        dp += b * b;
    }
    let denom = (dt * dp).sqrt();
    if denom == 0.0 {
        return 0.0;
    }
    num / denom
}

impl HudTemplate {
    pub fn from_png(bytes: &[u8], roi: (u32, u32, u32, u32), threshold: f64) -> Result<Self> {
        let img = image::load_from_memory(bytes)
            .map_err(|e| Error::Image(format!("could not decode HUD template PNG: {e}")))?;
        let (src_w, src_h) = (img.width(), img.height());
        let (x, y, w, h) = roi;
        if x + w > src_w || y + h > src_h || w == 0 || h == 0 {
            return Err(Error::Validation(format!(
                "HUD ROI {roi:?} does not fit inside a {src_w}x{src_h} image"
            )));
        }
        let rgb = img.to_rgb8();
        let mut bgr = Vec::with_capacity((src_w * src_h * 3) as usize);
        for px in rgb.pixels() {
            bgr.push(px[2]);
            bgr.push(px[1]);
            bgr.push(px[0]);
        }
        let gray = crop_gray_bgr(&bgr, src_w, roi);
        Ok(Self {
            roi,
            src: (src_w, src_h),
            gray,
            w,
            h,
            threshold,
        })
    }

    pub fn load(path: &Path, roi: (u32, u32, u32, u32), threshold: f64) -> Result<Self> {
        let bytes = std::fs::read(path).map_err(|e| {
            Error::Image(format!(
                "could not read HUD template {}: {e}",
                path.display()
            ))
        })?;
        Self::from_png(&bytes, roi, threshold)
    }

    pub fn score(&self, frame: &BgrFrame) -> Result<f64> {
        if frame.data.len() != (frame.width * frame.height * 3) as usize {
            return Err(Error::Image(format!(
                "BGR frame is {} bytes, expected {}",
                frame.data.len(),
                frame.width * frame.height * 3
            )));
        }
        let (rx, ry, rw, rh) = self.roi;
        let (sx, sy, sw, sh) = if (frame.width, frame.height) == self.src {
            (rx, ry, rw, rh)
        } else {
            let kx = frame.width as f64 / self.src.0 as f64;
            let ky = frame.height as f64 / self.src.1 as f64;
            (
                (rx as f64 * kx).round() as u32,
                (ry as f64 * ky).round() as u32,
                (rw as f64 * kx).round().max(1.0) as u32,
                (rh as f64 * ky).round().max(1.0) as u32,
            )
        };
        if sx + sw > frame.width || sy + sh > frame.height {
            return Err(Error::Image(format!(
                "HUD ROI does not fit inside a {}x{} frame",
                frame.width, frame.height
            )));
        }
        let patch = crop_gray_bgr(&frame.data, frame.width, (sx, sy, sw, sh));
        let template = if (sw, sh) == (self.w, self.h) {
            &self.gray
        } else {
            &resize_bilinear(&self.gray, self.w, self.h, sw, sh)
        };
        Ok(ncc(template, &patch))
    }

    pub fn matches(&self, frame: &BgrFrame) -> Result<bool> {
        Ok(self.score(frame)? >= self.threshold)
    }
}
