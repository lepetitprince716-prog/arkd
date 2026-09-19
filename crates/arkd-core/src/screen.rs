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
            let encoder =
                image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, opts.quality);
            img.write_with_encoder(encoder)
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
