//! Smart background removal in pure Rust.
//!
//! The subject is segmented with BiRefNet (Swin-T "lite" general weights)
//! running on a small built-in CPU inference engine, then the matte is refined
//! against the full-resolution image and the foreground colours are
//! decontaminated so hair and other fine edges blend cleanly onto any backdrop.
//!
//! ```no_run
//! let remover = bg_removal::BgRemover::from_file("birefnet-lite.safetensors")?;
//! let img = image::open("photo.jpg")?.to_rgba8();
//! let cutout = remover.remove_background(&img, &bg_removal::Options::default());
//! cutout.save("photo-cutout.png")?;
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

pub mod birefnet;
pub mod conv;
pub mod matting;
pub mod swin;
pub mod tensor;
pub mod weights;

use birefnet::{BiRefNet, INPUT_SIZE, MEAN, STD};
use image::{GrayImage, RgbaImage, imageops};
use rayon::prelude::*;
use tensor::Tensor;

pub use weights::WeightError;

/// What to do after the network has predicted the matte.
#[derive(Clone, Debug)]
pub struct Options {
    /// Snap the matte to the real edges of the full-resolution photo.
    pub refine_edges: bool,
    /// Re-estimate colours of semi-transparent pixels (hair, fur) so the old
    /// background does not shine through on a new one.
    pub decontaminate_colors: bool,
    /// Drop detached blobs much smaller than the main subject.
    pub main_subject_only: bool,
    /// Free the inference scratch memory after each call (on by default; turn
    /// it off when processing many images in a row).
    pub release_memory: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options { refine_edges: true, decontaminate_colors: true, main_subject_only: true, release_memory: true }
    }
}

pub struct BgRemover {
    net: BiRefNet,
}

impl BgRemover {
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, WeightError> {
        let mut w = weights::Weights::from_bytes(bytes)?;
        let net = BiRefNet::load(&mut w)?;
        Ok(BgRemover { net })
    }

    pub fn from_file(path: impl AsRef<std::path::Path>) -> Result<Self, WeightError> {
        let p = path.as_ref();
        let bytes = std::fs::read(p).map_err(|e| WeightError(format!("{}: {e}", p.display())))?;
        Self::from_bytes(&bytes)
    }

    /// Raw network matte at `INPUT_SIZE x INPUT_SIZE`, values in `[0, 1]`.
    pub fn predict_raw(&self, img: &RgbaImage) -> Vec<f32> {
        let small = imageops::resize(img, INPUT_SIZE as u32, INPUT_SIZE as u32, imageops::FilterType::Triangle);
        let n = INPUT_SIZE * INPUT_SIZE;
        let mut x = Tensor::zeros(&[3, INPUT_SIZE, INPUT_SIZE]);
        for (c, plane) in x.data.chunks_mut(n).enumerate() {
            for (v, px) in plane.iter_mut().zip(small.pixels()) {
                *v = (px[c] as f32 / 255.0 - MEAN[c]) / STD[c];
            }
        }
        let logits = self.net.forward(&x, None);
        logits.data.par_iter().map(|&v| tensor::sigmoid(v)).collect()
    }

    /// Alpha matte at the image's own resolution.
    pub fn alpha_mask(&self, img: &RgbaImage, opts: &Options) -> GrayImage {
        let (w, h) = (img.width(), img.height());
        let a = self.alpha_f32(img, opts, None);
        GrayImage::from_raw(w, h, a.iter().map(|v| (v * 255.0).round() as u8).collect()).unwrap()
    }

    fn alpha_f32(&self, img: &RgbaImage, opts: &Options, rgb: Option<&[f32]>) -> Vec<f32> {
        let (w, h) = (img.width() as usize, img.height() as usize);
        let mut pf = tensor::Prof::new();
        let mut raw = self.predict_raw(img);
        pf.lap("network");
        if opts.release_memory {
            tensor::pool::release();
        }
        if opts.main_subject_only {
            matting::keep_main_subject(&mut raw, INPUT_SIZE, INPUT_SIZE, 0.03);
        }
        let mut a = matting::resize_plane(&raw, INPUT_SIZE, INPUT_SIZE, w, h);
        if opts.refine_edges {
            let owned;
            let rgb = match rgb {
                Some(r) => r,
                None => {
                    owned = planar_rgb(img);
                    &owned
                }
            };
            a = matting::refine_alpha(rgb, &a, w, h);
        }
        pf.lap("subject+resize+refine");
        // respect transparency the source image already had
        a.par_iter_mut().zip(img.as_raw().par_chunks(4)).for_each(|(v, px)| *v *= px[3] as f32 / 255.0);
        a
    }

    /// Cut-out of the main subject with a transparent background.
    pub fn remove_background(&self, img: &RgbaImage, opts: &Options) -> RgbaImage {
        let (w, h) = (img.width() as usize, img.height() as usize);
        let n = w * h;
        let rgb = planar_rgb(img);
        let a = self.alpha_f32(img, opts, Some(&rgb));
        let mut pf = tensor::Prof::new();
        let fg = if opts.decontaminate_colors { matting::estimate_foreground(&rgb, &a, w, h) } else { rgb };
        pf.lap("foreground");
        let mut out = vec![0u8; n * 4];
        out.par_chunks_mut(4).enumerate().for_each(|(i, px)| {
            for c in 0..3 {
                px[c] = (fg[c * n + i] * 255.0 + 0.5).clamp(0.0, 255.0) as u8;
            }
            px[3] = (a[i] * 255.0 + 0.5) as u8;
        });
        RgbaImage::from_raw(w as u32, h as u32, out).unwrap()
    }
}

fn planar_rgb(img: &RgbaImage) -> Vec<f32> {
    let n = (img.width() * img.height()) as usize;
    let mut rgb = vec![0.0f32; 3 * n];
    let raw = img.as_raw();
    for c in 0..3 {
        rgb[c * n..(c + 1) * n].par_iter_mut().enumerate().for_each(|(i, v)| *v = raw[i * 4 + c] as f32 / 255.0);
    }
    rgb
}
