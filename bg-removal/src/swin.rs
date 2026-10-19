//! Swin Transformer v1 backbone (tiny configuration) as used by BiRefNet.

use crate::conv::Conv2d;
use crate::tensor::{pool, LayerNorm, Linear, Prof, Tensor, fast_exp, gelu_};
use crate::weights::{WeightError, Weights};
use rayon::prelude::*;

const WS: usize = 7;
const N: usize = WS * WS;
/// Head width of every Swin-T stage.
const HD: usize = 32;
/// `N` rounded up to a multiple of 8.
const NP: usize = 56;

struct Block {
    norm1: LayerNorm,
    qkv: Linear,
    /// `[heads, 49, 49]`
    rel_bias: Vec<f32>,
    proj: Linear,
    norm2: LayerNorm,
    fc1: Linear,
    fc2: Linear,
    heads: usize,
    shift: usize,
}

struct Stage {
    blocks: Vec<Block>,
    downsample: Option<(LayerNorm, Linear)>,
    out_norm: LayerNorm,
}

pub struct Swin {
    patch_proj: Conv2d,
    patch_norm: LayerNorm,
    stages: Vec<Stage>,
}

impl Swin {
    pub fn load(w: &mut Weights) -> Result<Self, WeightError> {
        let patch_proj = w.conv("bb.patch_embed.proj", 4, 0, 1, true)?;
        let patch_norm = w.layer_norm("bb.patch_embed.norm")?;
        let depths = [2, 2, 6, 2];
        let heads = [3, 6, 12, 24];
        let mut stages = Vec::new();
        for (s, (&depth, &nh)) in depths.iter().zip(&heads).enumerate() {
            let mut blocks = Vec::new();
            for b in 0..depth {
                let p = format!("bb.layers.{s}.blocks.{b}");
                blocks.push(Block {
                    norm1: w.layer_norm(&format!("{p}.norm1"))?,
                    qkv: w.linear(&format!("{p}.attn.qkv"), true)?,
                    rel_bias: w.vec(&format!("{p}.attn.rel_bias"))?,
                    proj: w.linear(&format!("{p}.attn.proj"), true)?,
                    norm2: w.layer_norm(&format!("{p}.norm2"))?,
                    fc1: w.linear(&format!("{p}.mlp.fc1"), true)?,
                    fc2: w.linear(&format!("{p}.mlp.fc2"), true)?,
                    heads: nh,
                    shift: if b % 2 == 1 { WS / 2 } else { 0 },
                });
            }
            let downsample = if s < 3 {
                Some((
                    w.layer_norm(&format!("bb.layers.{s}.downsample.norm"))?,
                    w.linear(&format!("bb.layers.{s}.downsample.reduction"), false)?,
                ))
            } else {
                None
            };
            stages.push(Stage { blocks, downsample, out_norm: w.layer_norm(&format!("bb.norm{s}"))? });
        }
        Ok(Swin { patch_proj, patch_norm, stages })
    }

    /// Returns the four stage outputs as `[C, H, W]` maps (strides 4, 8, 16, 32).
    pub fn forward(&self, x: &Tensor) -> Vec<Tensor> {
        let (_, ih, iw) = x.chw();
        assert!(ih % 4 == 0 && iw % 4 == 0, "input must be a multiple of 4");
        let emb = self.patch_proj.forward(x);
        let (_, mut h, mut w) = emb.chw();
        let mut t = emb.chw_to_tokens();
        self.patch_norm.forward_(&mut t);
        let mut outs = Vec::new();
        let mut prof = Prof::new();
        for st in &self.stages {
            for blk in &st.blocks {
                blk.forward(&mut t, h, w);
            }
            prof.lap(&format!("  stage {h}x{w}"));
            outs.push(st.out_norm.forward(&t).tokens_to_chw(h, w));
            if let Some((norm, red)) = &st.downsample {
                let (merged, nh, nw) = patch_merge(&t, h, w);
                let mut merged = merged;
                norm.forward_(&mut merged);
                t = red.forward(&merged);
                h = nh;
                w = nw;
            }
        }
        outs
    }
}

fn patch_merge(t: &Tensor, h: usize, w: usize) -> (Tensor, usize, usize) {
    let c = t.shape[1];
    let (nh, nw) = (h.div_ceil(2), w.div_ceil(2));
    let mut out = pool::zeros(nh * nw * 4 * c);
    out.par_chunks_mut(4 * c).enumerate().for_each(|(i, row)| {
        let (y, x) = (i / nw, i % nw);
        // torch order: x0 = (even, even), x1 = (odd, even), x2 = (even, odd), x3 = (odd, odd)
        for (q, (dy, dx)) in [(0, 0), (1, 0), (0, 1), (1, 1)].into_iter().enumerate() {
            let (sy, sx) = (2 * y + dy, 2 * x + dx);
            let dst = &mut row[q * c..(q + 1) * c];
            if sy < h && sx < w {
                dst.copy_from_slice(&t.data[(sy * w + sx) * c..(sy * w + sx + 1) * c]);
            }
        }
    });
    (Tensor::from_vec(&[nh * nw, 4 * c], out), nh, nw)
}

impl Block {
    fn forward(&self, x: &mut Tensor, h: usize, w: usize) {
        let c = x.shape[1];
        let hp = h.div_ceil(WS) * WS;
        let wp = w.div_ceil(WS) * WS;
        let (nwy, nwx) = (hp / WS, wp / WS);
        let nwin = nwy * nwx;
        let s = self.shift;
        // window-ordered token -> source token index in the unpadded grid (or None for padding)
        let src_of = |row: usize| -> Option<usize> {
            let (win, t) = (row / N, row % N);
            let gy = (win / nwx) * WS + t / WS;
            let gx = (win % nwx) * WS + t % WS;
            let (sy, sx) = ((gy + s) % hp, (gx + s) % wp);
            (sy < h && sx < w).then_some(sy * w + sx)
        };
        let mut pf = Prof::detail();
        let xn = self.norm1.forward(x);
        pf.lap("ln1");
        let mut xw = pool::zeros(nwin * N * c);
        xw.par_chunks_mut(c).enumerate().for_each(|(row, dst)| {
            if let Some(src) = src_of(row) {
                dst.copy_from_slice(&xn.data[src * c..(src + 1) * c]);
            }
        });
        drop(xn);
        pf.lap("gather");
        let qkv = self.qkv.forward(&Tensor::from_vec(&[nwin * N, c], xw));
        pf.lap("qkv");
        let attn = self.window_attention(&qkv, c, nwx, hp, wp);
        pf.lap("attn");
        drop(qkv);
        let proj = self.proj.forward(&attn);
        drop(attn);
        pf.lap("proj");
        // scatter back (reverse windows, undo the cyclic shift, crop padding) as a residual add
        let mut dst_of = vec![usize::MAX; h * w];
        for row in 0..nwin * N {
            if let Some(src) = src_of(row) {
                dst_of[src] = row;
            }
        }
        x.data.par_chunks_mut(c).enumerate().for_each(|(i, d)| {
            let r = dst_of[i];
            for (a, b) in d.iter_mut().zip(&proj.data[r * c..(r + 1) * c]) {
                *a += *b;
            }
        });
        drop(proj);
        pf.lap("scatter");
        let n2 = self.norm2.forward(x);
        pf.lap("ln2");
        let mut hdn = self.fc1.forward(&n2);
        pf.lap("fc1");
        gelu_(&mut hdn);
        pf.lap("gelu");
        let y = self.fc2.forward(&hdn);
        pf.lap("fc2");
        x.add_(&y);
        pf.lap("add");
    }

    fn window_attention(&self, qkv: &Tensor, c: usize, nwx: usize, hp: usize, wp: usize) -> Tensor {
        let nh = self.heads;
        let hd = c / nh;
        let scale = (hd as f32).powf(-0.5);
        let rows = qkv.shape[0];
        let s = self.shift;
        let region = |g: usize, len: usize| -> usize {
            if g < len - WS {
                0
            } else if g < len - s {
                1
            } else {
                2
            }
        };
        let mut out = pool::zeros(rows * c);
        out.par_chunks_mut(N * c).enumerate().for_each(|(win, dst)| {
            let base = win * N;
            let labels: Option<[u8; N]> = (s > 0).then(|| {
                let mut l = [0u8; N];
                for (t, v) in l.iter_mut().enumerate() {
                    let gy = (win / nwx) * WS + t / WS;
                    let gx = (win % nwx) * WS + t % WS;
                    *v = (region(gy, hp) * 3 + region(gx, wp)) as u8;
                }
                l
            });
            assert_eq!(hd, HD, "Swin-T uses 32-wide heads");
            // K transposed and padded to NP columns so every inner loop has a fixed, vector-friendly length
            let mut kt = [[0.0f32; NP]; HD];
            let mut sc = [[0.0f32; NP]; N];
            for head in 0..nh {
                let qo = head * HD;
                let ko = c + head * HD;
                let vo = 2 * c + head * HD;
                for j in 0..N {
                    let krow: &[f32; HD] = qkv.data[(base + j) * 3 * c + ko..][..HD].try_into().unwrap();
                    for d in 0..HD {
                        kt[d][j] = krow[d];
                    }
                }
                let bias = &self.rel_bias[head * N * N..(head + 1) * N * N];
                for (i, row) in sc.iter_mut().enumerate() {
                    let qrow: &[f32; HD] = qkv.data[(base + i) * 3 * c + qo..][..HD].try_into().unwrap();
                    let mut acc = [0.0f32; NP];
                    for d in 0..HD {
                        let qv = qrow[d] * scale;
                        for (a, b) in acc.iter_mut().zip(&kt[d]) {
                            *a += qv * b;
                        }
                    }
                    for j in 0..N {
                        row[j] = acc[j] + bias[i * N + j];
                    }
                    if let Some(l) = &labels {
                        for j in 0..N {
                            if l[j] != l[i] {
                                row[j] = f32::NEG_INFINITY;
                            }
                        }
                    }
                    let mut mx = f32::NEG_INFINITY;
                    for &v in &row[..N] {
                        mx = if v > mx { v } else { mx };
                    }
                    let mut sum = 0.0;
                    for v in row[..N].iter_mut() {
                        *v = fast_exp(*v - mx);
                        sum += *v;
                    }
                    let inv = 1.0 / sum;
                    for v in row[..N].iter_mut() {
                        *v *= inv;
                    }
                }
                for (i, row) in sc.iter().enumerate() {
                    let mut o = [0.0f32; HD];
                    for (j, &p) in row[..N].iter().enumerate() {
                        let vrow: &[f32; HD] = qkv.data[(base + j) * 3 * c + vo..][..HD].try_into().unwrap();
                        for d in 0..HD {
                            o[d] += p * vrow[d];
                        }
                    }
                    dst[i * c + qo..i * c + qo + HD].copy_from_slice(&o);
                }
            }
        });
        Tensor::from_vec(&[rows, c], out)
    }
}
