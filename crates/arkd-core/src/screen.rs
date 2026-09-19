use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::playtools::Frame;

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoordSpace {
    Screenshot,
    Device,
}

#[derive(Clone, Copy, Debug)]
pub struct Geometry {
    pub device: (u32, u32),
    pub screenshot: (u32, u32),
}

fn round_half_even(v: f64) -> i32 {
    let r = v.round();
    if (v - v.trunc()).abs() == 0.5 && (r as i64) % 2 != 0 {
        (r - r.signum()) as i32
    } else {
        r as i32
    }
}

impl Geometry {
    pub fn to_device(&self, x: f64, y: f64, space: CoordSpace) -> (i32, i32) {
        match space {
            CoordSpace::Device => (round_half_even(x), round_half_even(y)),
            CoordSpace::Screenshot => {
                let (dw, dh) = (self.device.0 as f64, self.device.1 as f64);
                let (sw, sh) = (self.screenshot.0 as f64, self.screenshot.1 as f64);
                (round_half_even(x * dw / sw), round_half_even(y * dh / sh))
            }
        }
    }

    /// Mirrors MaaCore's ControlScaleProxy: AsstAsyncClick takes coordinates in
    /// MaaCore's normalised click frame (1280x720 for 16:9-or-wider devices),
    /// not raw device pixels.
    pub fn to_maacore(&self, x: f64, y: f64, space: CoordSpace) -> (i32, i32) {
        let (dw, dh) = (self.device.0 as f64, self.device.1 as f64);
        let (sw, sh) = (self.screenshot.0 as f64, self.screenshot.1 as f64);
        let (dx, dy) = match space {
            CoordSpace::Device => (x, y),
            CoordSpace::Screenshot => (x * dw / sw, y * dh / sh),
        };
        let scale = if dw / dh >= 16.0 / 9.0 - 1e-7 {
            720.0 / dh
        } else {
            1280.0 / dw
        };
        (round_half_even(dx * scale), round_half_even(dy * scale))
    }

    pub fn to_screenshot(&self, x: f64, y: f64, space: CoordSpace) -> (i32, i32) {
        match space {
            CoordSpace::Screenshot => (round_half_even(x), round_half_even(y)),
            CoordSpace::Device => {
                let (dw, dh) = (self.device.0 as f64, self.device.1 as f64);
                let (sw, sh) = (self.screenshot.0 as f64, self.screenshot.1 as f64);
                (round_half_even(x * sw / dw), round_half_even(y * sh / dh))
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageFormat {
    Png,
    Jpeg,
}

#[derive(Clone, Copy, Debug)]
pub struct EncodeOpts {
    pub scale: f64,
    pub format: ImageFormat,
    pub quality: u8,
}

impl Default for EncodeOpts {
    fn default() -> Self {
        Self {
            scale: 1.0,
            format: ImageFormat::Png,
            quality: 80,
        }
    }
}

pub struct Encoded {
    pub bytes: Vec<u8>,
    pub mime: &'static str,
    pub width: u32,
    pub height: u32,
}

pub fn encode_png(png: &[u8], opts: EncodeOpts) -> Result<Encoded> {
    if (opts.scale - 1.0).abs() < f64::EPSILON && matches!(opts.format, ImageFormat::Png) {
        let dims = png_dimensions(png)?;
        return Ok(Encoded {
            bytes: png.to_vec(),
            mime: "image/png",
            width: dims.0,
            height: dims.1,
        });
    }
    let img =
        image::load_from_memory(png).map_err(|e| Error::Image(format!("decode failed: {e}")))?;
    let img = if (opts.scale - 1.0).abs() < f64::EPSILON {
        img
    } else {
        let w = ((img.width() as f64) * opts.scale).round().max(1.0) as u32;
        let h = ((img.height() as f64) * opts.scale).round().max(1.0) as u32;
        img.resize_exact(w, h, image::imageops::FilterType::Triangle)
    };
    encode_dynamic(&img, opts)
}

pub fn encode_frame(frame: &Frame, opts: EncodeOpts) -> Result<Encoded> {
    let png = frame.to_png()?;
    encode_png(&png, opts)
}

pub fn encode_frame_normalised(
    frame: &Frame,
    target: (u32, u32),
    opts: EncodeOpts,
) -> Result<Encoded> {
    let expected = frame.width as usize * frame.height as usize * 3;
    if frame.bgr.len() != expected {
        return Err(Error::Image(format!(
            "frame dimensions {}x{} do not match {} bytes",
            frame.width,
            frame.height,
            frame.bgr.len()
        )));
    }
    let mut rgb = Vec::with_capacity(expected);
    for px in frame.bgr.as_chunks::<3>().0 {
        rgb.extend_from_slice(&[px[2], px[1], px[0]]);
    }
    let img = image::RgbImage::from_raw(frame.width, frame.height, rgb).ok_or_else(|| {
        Error::Image(format!(
            "could not build a {}x{} image from {} bytes",
            frame.width,
            frame.height,
            frame.bgr.len()
        ))
    })?;
    let out_w = ((target.0 as f64) * opts.scale).round().max(1.0) as u32;
    let out_h = ((target.1 as f64) * opts.scale).round().max(1.0) as u32;
    let mut img = image::DynamicImage::ImageRgb8(img);
    if (frame.width, frame.height) != (out_w, out_h) {
        img = img.resize_exact(out_w, out_h, image::imageops::FilterType::Triangle);
    }
    encode_dynamic(&img, opts)
}

fn encode_dynamic(img: &image::DynamicImage, opts: EncodeOpts) -> Result<Encoded> {
    let mut out = Vec::new();
    match opts.format {
        ImageFormat::Png => {
            let encoder = image::codecs::png::PngEncoder::new(&mut out);
            image::ImageEncoder::write_image(
                encoder,
                img.as_bytes(),
                img.width(),
                img.height(),
                img.color().into(),
            )
            .map_err(|e| Error::Image(format!("PNG encode failed: {e}")))?;
        }
        ImageFormat::Jpeg => {
            let rgb = img.to_rgb8();
            let encoder =
                image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, opts.quality);
            image::ImageEncoder::write_image(
                encoder,
                rgb.as_raw(),
                rgb.width(),
                rgb.height(),
                image::ExtendedColorType::Rgb8,
            )
            .map_err(|e| Error::Image(format!("JPEG encode failed: {e}")))?;
        }
    }
    Ok(Encoded {
        bytes: out,
        mime: match opts.format {
            ImageFormat::Png => "image/png",
            ImageFormat::Jpeg => "image/jpeg",
        },
        width: img.width(),
        height: img.height(),
    })
}

pub fn png_dimensions(png: &[u8]) -> Result<(u32, u32)> {
    image::ImageReader::new(std::io::Cursor::new(png))
        .with_guessed_format()
        .map_err(|e| Error::Image(format!("not a readable image: {e}")))?
        .into_dimensions()
        .map_err(|e| Error::Image(format!("could not read dimensions: {e}")))
}
