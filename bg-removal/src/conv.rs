//! 2-D convolutions (plain and modulated deformable).
//!
//! Both are computed per block of output pixels on a channels-last copy of the
//! input: each worker fills a small, cache-resident im2col buffer
//! (`[pixels, taps * cin]`) and runs a single-threaded GEMM into its own
//! columns of the output. Channels-last makes every tap a contiguous run of
//! `cin` floats, which turns both the copies and the deformable bilinear
//! sampling into vectorised loops.

use crate::tensor::{SendPtr, Tensor, gemm_strided, pool, sgemm, sigmoid};
use std::cell::RefCell;
use rayon::prelude::*;

pub struct Conv2d {
    /// `[cout, cin * k * k]` in PyTorch order (channel-major).
    pub w: Vec<f32>,
    /// Same weights reordered tap-major: `[cout, k * k * cin]`.
    w_kc: Vec<f32>,
    pub b: Option<Vec<f32>>,
    pub cin: usize,
    pub cout: usize,
    pub k: usize,
    pub stride: usize,
    pub pad: usize,
    pub dil: usize,
}

/// Target im2col buffer size per worker, in floats.
const BLOCK_FLOATS: usize = 1 << 18;

fn block_pixels(row_len: usize, n: usize) -> usize {
    let threads = rayon::current_num_threads();
    let p = (BLOCK_FLOATS / row_len).clamp(32, 512) / 16 * 16;
    // keep enough blocks for all workers on small maps
    let max_for_par = (n / (2 * threads)).max(16) / 16 * 16;
    p.min(max_for_par.max(16))
}

thread_local! {
    static SCRATCH: RefCell<Vec<f32>> = const { RefCell::new(Vec::new()) };
}

/// Runs `f` on this worker's reusable im2col buffer of at least `len` floats.
fn with_scratch<R>(len: usize, f: impl FnOnce(&mut [f32]) -> R) -> R {
    SCRATCH.with(|s| {
        let mut s = s.borrow_mut();
        if s.len() < len {
            s.resize(len, 0.0);
        }
        f(&mut s[..len])
    })
}

pub fn to_hwc(x: &Tensor) -> Vec<f32> {
    let (c, h, w) = x.chw();
    let n = h * w;
    let mut out = pool::zeros(n * c);
    // transpose in tiles of 64 pixels to stay cache friendly
    out.par_chunks_mut(64 * c).enumerate().for_each(|(bi, dst)| {
        let p0 = bi * 64;
        let pn = dst.len() / c;
        for ci in 0..c {
            let src = &x.data[ci * n + p0..ci * n + p0 + pn];
            for (q, v) in src.iter().enumerate() {
                dst[q * c + ci] = *v;
            }
        }
    });
    out
}

fn reorder_tap_major(w: &[f32], cout: usize, cin: usize, kk: usize) -> Vec<f32> {
    let mut out = vec![0.0; w.len()];
    for o in 0..cout {
        for c in 0..cin {
            for t in 0..kk {
                out[o * cin * kk + t * cin + c] = w[o * cin * kk + c * kk + t];
            }
        }
    }
    out
}

/// Writes `cout x pn` results for one pixel block: `out[:, p0..p0+pn] (+)= W * colᵀ`.
fn block_product(w: &[f32], cout: usize, r: usize, col: &[f32], pn: usize, out: SendPtr, n: usize, p0: usize) {
    if cout <= 2 {
        for o in 0..cout {
            let wr = &w[o * r..(o + 1) * r];
            for q in 0..pn {
                let cr = &col[q * r..(q + 1) * r];
                let mut acc = [0.0f32; 8];
                let mut i = 0;
                while i + 8 <= r {
                    for l in 0..8 {
                        acc[l] += wr[i + l] * cr[i + l];
                    }
                    i += 8;
                }
                let mut s: f32 = acc.iter().sum();
                while i < r {
                    s += wr[i] * cr[i];
                    i += 1;
                }
                // SAFETY: (o, p0 + q) is inside this block's own output columns.
                unsafe { *out.0.add(o * n + p0 + q) += s };
            }
        }
    } else {
        // SAFETY: W is cout x r, col holds pn rows of r, the output block is cout x pn at p0.
        unsafe {
            gemm_strided(cout, pn, r, w.as_ptr(), r, 1, col.as_ptr(), 1, r, out.0.add(p0), n, 1, true);
        }
    }
}

impl Conv2d {
    #[allow(clippy::too_many_arguments)]
    pub fn new(w: Vec<f32>, b: Option<Vec<f32>>, cout: usize, cin: usize, k: usize, stride: usize, pad: usize, dil: usize) -> Self {
        assert_eq!(w.len(), cout * cin * k * k);
        let w_kc = reorder_tap_major(&w, cout, cin, k * k);
        Conv2d { w, w_kc, b, cin, cout, k, stride, pad, dil }
    }

    pub fn out_hw(&self, h: usize, w: usize) -> (usize, usize) {
        let eff = self.dil * (self.k - 1) + 1;
        ((h + 2 * self.pad - eff) / self.stride + 1, (w + 2 * self.pad - eff) / self.stride + 1)
    }

    pub fn forward(&self, x: &Tensor) -> Tensor {
        let (_, h, w) = x.chw();
        let (oh, ow) = self.out_hw(h, w);
        let mut out = Tensor::zeros(&[self.cout, oh, ow]);
        self.forward_into(x, &mut out.data, false);
        out
    }

    /// Writes (or with `accumulate` adds) the convolution into `out` (`[cout, oh*ow]`), bias included.
    pub fn forward_into(&self, x: &Tensor, out: &mut [f32], accumulate: bool) {
        let (c, h, w) = x.chw();
        assert_eq!(c, self.cin, "conv input channels");
        if self.k == 1 && self.stride == 1 && self.pad == 0 {
            let n = h * w;
            self.init_bias(out, n, accumulate);
            sgemm(self.cout, n, c, &self.w, c, &x.data, n, out, n, true);
            return;
        }
        let xh = to_hwc(x);
        self.forward_hwc(&xh, h, w, out, accumulate);
    }

    fn init_bias(&self, out: &mut [f32], n: usize, accumulate: bool) {
        assert_eq!(out.len(), self.cout * n);
        match (&self.b, accumulate) {
            (Some(b), _) => out.par_chunks_mut(n).zip(b.par_iter()).for_each(|(ch, &bv)| {
                if accumulate {
                    ch.iter_mut().for_each(|v| *v += bv);
                } else {
                    ch.fill(bv);
                }
            }),
            (None, false) => out.par_iter_mut().for_each(|v| *v = 0.0),
            (None, true) => {}
        }
    }

    /// Same as [`forward_into`](Self::forward_into) on a channels-last input.
    pub fn forward_hwc(&self, xh: &[f32], h: usize, w: usize, out: &mut [f32], accumulate: bool) {
        let cin = self.cin;
        let (oh, ow) = self.out_hw(h, w);
        let n = oh * ow;
        self.init_bias(out, n, accumulate);
        let r = self.k * self.k * cin;
        let pblk = block_pixels(r, n);
        let outp = SendPtr(out.as_mut_ptr());
        (0..n.div_ceil(pblk)).into_par_iter().for_each(|bi| {
            with_scratch(pblk * r, |col| {
                let p0 = bi * pblk;
                let pn = pblk.min(n - p0);
                for q in 0..pn {
                    let (oy, ox) = ((p0 + q) / ow, (p0 + q) % ow);
                    let dst = &mut col[q * r..(q + 1) * r];
                    for ky in 0..self.k {
                        let iy = (oy * self.stride + ky * self.dil) as isize - self.pad as isize;
                        for kx in 0..self.k {
                            let ix = (ox * self.stride + kx * self.dil) as isize - self.pad as isize;
                            let d = &mut dst[(ky * self.k + kx) * cin..][..cin];
                            if iy >= 0 && iy < h as isize && ix >= 0 && ix < w as isize {
                                let s = (iy as usize * w + ix as usize) * cin;
                                d.copy_from_slice(&xh[s..s + cin]);
                            } else {
                                d.fill(0.0);
                            }
                        }
                    }
                }
                block_product(&self.w_kc, self.cout, r, col, pn, outp, n, p0);
            })
        });
    }
}

/// Modulated deformable convolution (DCNv2, torchvision `deform_conv2d`
/// semantics), stride 1 and "same" padding.
pub struct DeformConv2d {
    /// Offsets (`2*k*k` channels) and modulator (`k*k` channels) fused into one conv.
    pub offset_mod: Conv2d,
    /// `[cout, k * k * cin]`, tap-major, BatchNorm already folded in.
    w_kc: Vec<f32>,
    pub b: Vec<f32>,
    pub cin: usize,
    pub cout: usize,
    pub k: usize,
}

impl DeformConv2d {
    pub fn new(offset_mod: Conv2d, w: Vec<f32>, b: Vec<f32>, cout: usize, cin: usize, k: usize) -> Self {
        let w_kc = reorder_tap_major(&w, cout, cin, k * k);
        DeformConv2d { offset_mod, w_kc, b, cin, cout, k }
    }

    /// `relu(deform_conv(x))` when `relu`.
    pub fn forward(&self, x: &Tensor, xh: &[f32], relu: bool) -> Tensor {
        let (c, h, w) = x.chw();
        assert_eq!(c, self.cin);
        let n = h * w;
        let k = self.k;
        let kk = k * k;
        let cin = self.cin;
        let pad = (k / 2) as f32;
        let mut om = pool::zeros(3 * kk * n);
        if k == 1 {
            self.offset_mod.forward_into(x, &mut om, false);
        } else {
            self.offset_mod.forward_hwc(xh, h, w, &mut om, false);
        }
        let (off, modu) = om.split_at(2 * kk * n);

        let mut out = pool::zeros(self.cout * n);
        out.par_chunks_mut(n).zip(self.b.par_iter()).for_each(|(ch, &b)| ch.fill(b));
        let r = kk * cin;
        let pblk = block_pixels(r, n);
        let outp = SendPtr(out.as_mut_ptr());
        (0..n.div_ceil(pblk)).into_par_iter().for_each(|bi| {
            with_scratch(pblk * r, |col| {
                let p0 = bi * pblk;
                let pn = pblk.min(n - p0);
                for q in 0..pn {
                    let g = p0 + q;
                    let (yy, xx) = ((g / w) as f32, (g % w) as f32);
                    let dst = &mut col[q * r..(q + 1) * r];
                    for t in 0..kk {
                        let py = yy - pad + (t / k) as f32 + off[2 * t * n + g];
                        let px = xx - pad + (t % k) as f32 + off[(2 * t + 1) * n + g];
                        let m = 2.0 * sigmoid(modu[t * n + g]);
                        let d = &mut dst[t * cin..(t + 1) * cin];
                        sample_bilinear(xh, h, w, cin, py, px, m, d);
                    }
                }
                block_product(&self.w_kc, self.cout, r, col, pn, outp, n, p0);
            })
        });
        pool::give(om);
        let mut t = Tensor::from_vec(&[self.cout, h, w], out);
        if relu {
            t.relu_();
        }
        t
    }
}

/// torchvision's `bilinear_interpolate` (zero outside, per-corner validity) applied
/// to all channels of a channels-last map at once, scaled by `m`.
#[inline]
#[allow(clippy::too_many_arguments)]
fn sample_bilinear(xh: &[f32], h: usize, w: usize, cin: usize, y: f32, x: f32, m: f32, d: &mut [f32]) {
    d.fill(0.0);
    if y <= -1.0 || y >= h as f32 || x <= -1.0 || x >= w as f32 {
        return;
    }
    // y, x > -1 here, so truncation fixed up for negatives is floor (and stays vectorisable)
    let y0 = if y < 0.0 { -1.0 } else { (y as i32) as f32 };
    let x0 = if x < 0.0 { -1.0 } else { (x as i32) as f32 };
    let (ly, lx) = (y - y0, x - x0);
    let (hy, hx) = (1.0 - ly, 1.0 - lx);
    let (y0, x0) = (y0 as isize, x0 as isize);
    let (hi, wi) = (h as isize, w as isize);
    let corners = [(y0, x0, hy * hx), (y0, x0 + 1, hy * lx), (y0 + 1, x0, ly * hx), (y0 + 1, x0 + 1, ly * lx)];
    for (cy, cx, cw) in corners {
        if cy >= 0 && cy < hi && cx >= 0 && cx < wi {
            let s = (cy * wi + cx) as usize * cin;
            let cw = cw * m;
            for (a, b) in d.iter_mut().zip(&xh[s..s + cin]) {
                *a += cw * b;
            }
        }
    }
}
