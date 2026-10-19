//! Minimal dense f32 tensor and the CPU kernels the network needs.
//!
//! Feature maps are stored as `[C, H, W]` (batch size is always 1), token
//! sequences as `[L, C]`, both row-major.

use rayon::prelude::*;

#[derive(Debug)]
pub struct Tensor {
    pub shape: Vec<usize>,
    pub data: Vec<f32>,
}

/// Recycles large activation buffers between layers. Freshly allocated memory
/// costs a page fault per 4 KiB on first touch, which for a network that
/// streams several GiB of activations is a noticeable share of the run time.
pub mod pool {
    use std::sync::Mutex;

    static POOL: Mutex<Vec<Vec<f32>>> = Mutex::new(Vec::new());
    const MIN_POOLED: usize = 1 << 16;

    /// A zero-filled buffer of `len` floats.
    pub fn zeros(len: usize) -> Vec<f32> {
        let mut v = take(len);
        v.resize(len, 0.0);
        v
    }

    /// An empty buffer with capacity for at least `len` floats.
    pub fn take(len: usize) -> Vec<f32> {
        if len >= MIN_POOLED {
            let mut p = POOL.lock().unwrap();
            let best = p
                .iter()
                .enumerate()
                .filter(|(_, v)| v.capacity() >= len && v.capacity() <= 2 * len)
                .min_by_key(|(_, v)| v.capacity())
                .map(|(i, _)| i);
            if let Some(i) = best {
                let mut v = p.swap_remove(i);
                v.clear();
                return v;
            }
        }
        Vec::with_capacity(len)
    }

    pub fn give(v: Vec<f32>) {
        if v.capacity() >= MIN_POOLED {
            POOL.lock().unwrap().push(v);
        }
    }

    /// Frees every pooled buffer.
    pub fn release() {
        POOL.lock().unwrap().clear();
    }
}

impl Drop for Tensor {
    fn drop(&mut self) {
        pool::give(std::mem::take(&mut self.data));
    }
}

impl Clone for Tensor {
    fn clone(&self) -> Self {
        let mut data = pool::take(self.data.len());
        data.extend_from_slice(&self.data);
        Tensor { shape: self.shape.clone(), data }
    }
}

impl Tensor {
    pub fn zeros(shape: &[usize]) -> Self {
        Tensor { shape: shape.to_vec(), data: pool::zeros(shape.iter().product()) }
    }

    pub fn from_vec(shape: &[usize], data: Vec<f32>) -> Self {
        assert_eq!(shape.iter().product::<usize>(), data.len(), "shape {shape:?}");
        Tensor { shape: shape.to_vec(), data }
    }

    pub fn len(&self) -> usize {
        self.data.len()
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    /// `(C, H, W)` of a feature map.
    pub fn chw(&self) -> (usize, usize, usize) {
        assert_eq!(self.shape.len(), 3, "expected [C,H,W], got {:?}", self.shape);
        (self.shape[0], self.shape[1], self.shape[2])
    }

    pub fn relu_(&mut self) {
        self.data.par_iter_mut().for_each(|v| *v = v.max(0.0));
    }

    pub fn sigmoid_(&mut self) {
        self.data.par_chunks_mut(4096).for_each(|ch| ch.iter_mut().for_each(|v| *v = sigmoid(*v)));
    }

    pub fn add_(&mut self, o: &Tensor) {
        assert_eq!(self.shape, o.shape);
        self.data.par_iter_mut().zip(o.data.par_iter()).for_each(|(a, b)| *a += *b);
    }

    /// Multiplies every channel of a `[C,H,W]` map by a `[1,H,W]` map.
    pub fn mul_spatial_(&mut self, m: &Tensor) {
        let (_, h, w) = self.chw();
        assert_eq!(m.shape, [1, h, w]);
        self.data.par_chunks_mut(h * w).for_each(|ch| {
            for (a, b) in ch.iter_mut().zip(&m.data) {
                *a *= *b;
            }
        });
    }

    /// `[L, C]` tokens laid out on an `h x w` grid to a `[C, h, w]` map.
    pub fn tokens_to_chw(&self, h: usize, w: usize) -> Tensor {
        let (l, c) = (self.shape[0], self.shape[1]);
        assert_eq!(l, h * w);
        let mut out = pool::zeros(l * c);
        out.par_chunks_mut(l).enumerate().for_each(|(ci, plane)| {
            for (p, v) in plane.iter_mut().enumerate() {
                *v = self.data[p * c + ci];
            }
        });
        Tensor::from_vec(&[c, h, w], out)
    }

    /// `[C, H, W]` map to `[H*W, C]` tokens.
    pub fn chw_to_tokens(&self) -> Tensor {
        let (c, h, w) = self.chw();
        let l = h * w;
        let mut out = pool::zeros(l * c);
        out.par_chunks_mut(c).enumerate().for_each(|(p, row)| {
            for (ci, v) in row.iter_mut().enumerate() {
                *v = self.data[ci * l + p];
            }
        });
        Tensor::from_vec(&[l, c], out)
    }
}

#[inline]
pub fn sigmoid(x: f32) -> f32 {
    1.0 / (1.0 + fast_exp(-x))
}

/// `exp(x)` with ~2 ulp error, written so that LLVM can vectorise loops over it
/// (the libm call cannot be).
#[inline(always)]
pub fn fast_exp(x: f32) -> f32 {
    let x = x.clamp(-87.0, 88.0);
    // round-to-nearest via the 1.5 * 2^23 trick: `f32::round` is a libm call on baseline x86-64
    const MAGIC: f32 = 12_582_912.0;
    let n = (x * std::f32::consts::LOG2_E + MAGIC) - MAGIC;
    let r = x - n * 0.693_359_4 + n * 2.121_944_4e-4;
    let r2 = r * r;
    let p = 1.987_569_1e-4;
    let p = p * r + 1.398_199_9e-3;
    let p = p * r + 8.333_452e-3;
    let p = p * r + 4.166_579_6e-2;
    let p = p * r + 1.666_666_5e-1;
    let p = p * r + 5.000_000_1e-1;
    let y = p * r2 + r + 1.0;
    y * f32::from_bits(((n as i32 + 127) << 23) as u32)
}

/// `erf(x)`, Abramowitz & Stegun 7.1.26 (|error| < 1.5e-7), vectorisable.
#[inline(always)]
pub fn fast_erf(x: f32) -> f32 {
    let a = x.abs();
    let t = 1.0 / (1.0 + 0.327_591_1 * a);
    let poly = t * (0.254_829_6 + t * (-0.284_496_74 + t * (1.421_413_7 + t * (-1.453_152_1 + t * 1.061_405_4))));
    let y = 1.0 - poly * fast_exp(-a * a);
    y.copysign(x)
}

/// Concatenates `[C_i, H, W]` maps along channels.
pub fn cat_channels(parts: &[&Tensor]) -> Tensor {
    let (_, h, w) = parts[0].chw();
    let c: usize = parts.iter().map(|t| t.shape[0]).sum();
    let mut data = pool::take(c * h * w);
    for t in parts {
        assert_eq!(&t.shape[1..], &[h, w]);
        data.extend_from_slice(&t.data);
    }
    Tensor::from_vec(&[c, h, w], data)
}

/// Raw pointer that may be shared across rayon tasks writing disjoint regions.
#[derive(Clone, Copy)]
pub struct SendPtr(pub *mut f32);
unsafe impl Send for SendPtr {}
unsafe impl Sync for SendPtr {}

/// Column block width for splitting wide products across threads.
const NB: usize = 512;
/// Row block height for splitting tall products across threads.
const MB: usize = 256;

/// `C[m,n] (+)= A[m,k] * B[k,n]`, all row-major with the given row strides.
///
/// Large products are split into independent blocks that run single-threaded
/// GEMMs in parallel; this scales much better than one multithreaded GEMM for
/// the very wide (pixels) or very tall (tokens) shapes of this network.
#[allow(clippy::too_many_arguments)]
pub fn sgemm(
    m: usize,
    n: usize,
    k: usize,
    a: &[f32],
    lda: usize,
    b: &[f32],
    ldb: usize,
    c: &mut [f32],
    ldc: usize,
    accumulate: bool,
) {
    if m == 0 || n == 0 {
        return;
    }
    assert!(a.len() >= (m - 1) * lda + k);
    assert!(b.len() >= (k - 1) * ldb + n);
    assert!(c.len() >= (m - 1) * ldc + n);
    let threads = rayon::current_num_threads();
    let cp = SendPtr(c.as_mut_ptr());
    if n >= 2 * NB * threads.min(4) && n >= m {
        (0..n.div_ceil(NB)).into_par_iter().for_each(|i| {
            let j0 = i * NB;
            let nb = NB.min(n - j0);
            let cp = cp;
            // SAFETY: each task writes its own column range of C, bounds checked above.
            let cs = unsafe { std::slice::from_raw_parts_mut(cp.0.add(j0), (m - 1) * ldc + nb) };
            sgemm_par(m, nb, k, a, lda, &b[j0..], ldb, cs, ldc, accumulate, gemm::Parallelism::None);
        });
    } else if m >= 2 * MB * threads.min(4) {
        (0..m.div_ceil(MB)).into_par_iter().for_each(|i| {
            let i0 = i * MB;
            let mb = MB.min(m - i0);
            let cp = cp;
            // SAFETY: each task writes its own row range of C, bounds checked above.
            let cs = unsafe { std::slice::from_raw_parts_mut(cp.0.add(i0 * ldc), (mb - 1) * ldc + n) };
            sgemm_par(mb, n, k, &a[i0 * lda..], lda, b, ldb, cs, ldc, accumulate, gemm::Parallelism::None);
        });
    } else {
        let par = if m * n * k < 1 << 20 { gemm::Parallelism::None } else { gemm::Parallelism::Rayon(threads) };
        sgemm_par(m, n, k, a, lda, b, ldb, c, ldc, accumulate, par);
    }
}

/// Single-threaded `C (+)= A * B` with explicit (row, column) strides for every operand.
#[allow(clippy::too_many_arguments)]
pub(crate) unsafe fn gemm_strided(
    m: usize,
    n: usize,
    k: usize,
    a: *const f32,
    a_rs: usize,
    a_cs: usize,
    b: *const f32,
    b_rs: usize,
    b_cs: usize,
    c: *mut f32,
    c_rs: usize,
    c_cs: usize,
    accumulate: bool,
) {
    unsafe {
        gemm::gemm(
            m,
            n,
            k,
            c,
            c_cs as isize,
            c_rs as isize,
            accumulate,
            a,
            a_cs as isize,
            a_rs as isize,
            b,
            b_cs as isize,
            b_rs as isize,
            1.0f32,
            1.0f32,
            false,
            false,
            false,
            gemm::Parallelism::None,
        );
    }
}

#[allow(clippy::too_many_arguments)]
pub fn sgemm_par(
    m: usize,
    n: usize,
    k: usize,
    a: &[f32],
    lda: usize,
    b: &[f32],
    ldb: usize,
    c: &mut [f32],
    ldc: usize,
    accumulate: bool,
    par: gemm::Parallelism,
) {
    if m == 0 || n == 0 {
        return;
    }
    assert!(a.len() >= (m - 1) * lda + k);
    assert!(b.len() >= (k - 1) * ldb + n);
    assert!(c.len() >= (m - 1) * ldc + n);
    // SAFETY: bounds of all three operands are checked above.
    unsafe {
        gemm::gemm(
            m,
            n,
            k,
            c.as_mut_ptr(),
            1,
            ldc as isize,
            accumulate,
            a.as_ptr(),
            1,
            lda as isize,
            b.as_ptr(),
            1,
            ldb as isize,
            1.0f32,
            1.0f32,
            false,
            false,
            false,
            par,
        );
    }
}

/// Fully connected layer on `[L, in]` tokens; weights are stored transposed as `[in, out]`.
pub struct Linear {
    pub w_t: Vec<f32>,
    pub b: Option<Vec<f32>>,
    pub cin: usize,
    pub cout: usize,
}

impl Linear {
    pub fn forward(&self, x: &Tensor) -> Tensor {
        let l = x.shape[0];
        assert_eq!(x.shape[1], self.cin);
        let mut out = pool::zeros(l * self.cout);
        if let Some(b) = &self.b {
            out.par_chunks_mut(self.cout).for_each(|row| row.copy_from_slice(b));
        }
        sgemm(l, self.cout, self.cin, &x.data, self.cin, &self.w_t, self.cout, &mut out, self.cout, self.b.is_some());
        Tensor::from_vec(&[l, self.cout], out)
    }
}

pub struct LayerNorm {
    pub w: Vec<f32>,
    pub b: Vec<f32>,
}

impl LayerNorm {
    pub fn forward(&self, x: &Tensor) -> Tensor {
        let mut y = x.clone();
        self.forward_(&mut y);
        y
    }

    pub fn forward_(&self, x: &mut Tensor) {
        let c = self.w.len();
        assert_eq!(*x.shape.last().unwrap(), c);
        x.data.par_chunks_mut(c).for_each(|row| {
            let mean = row.iter().sum::<f32>() / c as f32;
            let var = row.iter().map(|v| (v - mean) * (v - mean)).sum::<f32>() / c as f32;
            let inv = 1.0 / (var + 1e-5).sqrt();
            for ((v, w), b) in row.iter_mut().zip(&self.w).zip(&self.b) {
                *v = (*v - mean) * inv * w + b;
            }
        });
    }
}

pub fn gelu_(x: &mut Tensor) {
    x.data.par_chunks_mut(4096).for_each(|ch| {
        for v in ch {
            *v = 0.5 * *v * (1.0 + fast_erf(*v * std::f32::consts::FRAC_1_SQRT_2));
        }
    });
}

/// Bilinear resize of a `[C,H,W]` map, `align_corners=True` semantics.
pub fn interp_bilinear_ac(x: &Tensor, oh: usize, ow: usize) -> Tensor {
    let (c, h, w) = x.chw();
    if (h, w) == (oh, ow) {
        return x.clone();
    }
    let taps = |o: usize, i: usize| -> Vec<(usize, usize, f32)> {
        (0..o)
            .map(|d| {
                let s = if o > 1 { d as f32 * (i - 1) as f32 / (o - 1) as f32 } else { 0.0 };
                let i0 = (s.floor() as usize).min(i - 1);
                let i1 = (i0 + 1).min(i - 1);
                (i0, i1, s - i0 as f32)
            })
            .collect()
    };
    let ty = taps(oh, h);
    let tx = taps(ow, w);
    let mut out = pool::zeros(c * oh * ow);
    out.par_chunks_mut(oh * ow).enumerate().for_each(|(ci, dst)| {
        let src = &x.data[ci * h * w..(ci + 1) * h * w];
        for (oy, &(y0, y1, fy)) in ty.iter().enumerate() {
            let r0 = &src[y0 * w..(y0 + 1) * w];
            let r1 = &src[y1 * w..(y1 + 1) * w];
            let d = &mut dst[oy * ow..(oy + 1) * ow];
            for (v, &(x0, x1, fx)) in d.iter_mut().zip(&tx) {
                let a = r0[x0] + (r0[x1] - r0[x0]) * fx;
                let b = r1[x0] + (r1[x1] - r1[x0]) * fx;
                *v = a + (b - a) * fy;
            }
        }
    });
    Tensor::from_vec(&[c, oh, ow], out)
}

/// Coarse wall-clock profiling, enabled with `BG_PROFILE=1`.
pub struct Prof {
    on: bool,
    t: std::time::Instant,
}

impl Prof {
    pub fn new() -> Self {
        Prof { on: std::env::var_os("BG_PROFILE").is_some(), t: std::time::Instant::now() }
    }

    pub fn detail() -> Self {
        Prof { on: std::env::var("BG_PROFILE").as_deref() == Ok("2"), t: std::time::Instant::now() }
    }

    pub fn lap(&mut self, what: &str) {
        if self.on {
            eprintln!("  {what:28} {:8.1} ms", self.t.elapsed().as_secs_f64() * 1e3);
            self.t = std::time::Instant::now();
        }
    }
}

impl Default for Prof {
    fn default() -> Self {
        Self::new()
    }
}
