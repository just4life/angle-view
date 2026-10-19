//! BiRefNet (swin_v1_t backbone, `General` lite weights) forward pass.
//!
//! Mirrors `models/birefnet.py` of the reference implementation with the
//! configuration the published lite weights were trained with: multi-scale input
//! (`mul_scl_ipt = cat`), 3 context features, `BasicDecBlk` + `ASPPDeformable`
//! decoder blocks, split image patches fed into each decoder stage and
//! gradient-attention gating. Training-only heads are not evaluated.

use crate::conv::{Conv2d, DeformConv2d, to_hwc};
use crate::swin::Swin;
use crate::tensor::{Prof, Tensor, pool, cat_channels, interp_bilinear_ac, sgemm};
use crate::weights::{WeightError, Weights};
use rayon::prelude::*;
use std::collections::HashMap;

pub const INPUT_SIZE: usize = 1024;
pub const MEAN: [f32; 3] = [0.485, 0.456, 0.406];
pub const STD: [f32; 3] = [0.229, 0.224, 0.225];

struct AsppDeformable {
    /// aspp1 followed by aspp_deforms (kernel sizes 1, 1, 3, 7)
    branches: Vec<DeformConv2d>,
    gap_conv: Conv2d,
    conv1: Conv2d,
}

impl AsppDeformable {
    fn load(w: &mut Weights, p: &str) -> Result<Self, WeightError> {
        let mut branches = vec![load_deform(w, &format!("{p}.aspp1"))?];
        for i in 0..3 {
            branches.push(load_deform(w, &format!("{p}.aspp_deforms.{i}"))?);
        }
        Ok(AsppDeformable {
            branches,
            gap_conv: w.conv(&format!("{p}.global_avg_pool.1"), 1, 0, 1, true)?,
            conv1: w.conv(&format!("{p}.conv1"), 1, 0, 1, true)?,
        })
    }

    /// `relu(conv1(cat(branches..., gap)))`, accumulated branch by branch so the
    /// 1280-channel concatenation is never materialised.
    fn forward(&self, x: &Tensor) -> Tensor {
        let (c, h, w) = x.chw();
        let n = h * w;
        let co = self.conv1.cout;
        let ci = self.conv1.cin; // 5 * 256
        let bc = self.gap_conv.cout;
        // global average pool branch is spatially constant: fold it into the bias
        let mean: Vec<f32> = x.data.par_chunks(n).map(|p| p.iter().sum::<f32>() / n as f32).collect();
        let mut g = self.gap_conv.forward(&Tensor::from_vec(&[c, 1, 1], mean));
        g.relu_();
        let mut bias = self.conv1.b.clone().unwrap_or_else(|| vec![0.0; co]);
        for (o, b) in bias.iter_mut().enumerate() {
            let wrow = &self.conv1.w[o * ci + 4 * bc..o * ci + 5 * bc];
            *b += wrow.iter().zip(&g.data).map(|(a, b)| a * b).sum::<f32>();
        }
        let mut out = pool::zeros(co * n);
        out.par_chunks_mut(n).zip(bias.par_iter()).for_each(|(ch, &b)| ch.fill(b));
        let mut pf = Prof::detail();
        let xh = to_hwc(x);
        pf.lap("aspp hwc+gap");
        for (i, br) in self.branches.iter().enumerate() {
            let y = br.forward(x, &xh, true);
            pf.lap(["aspp br1x1", "aspp br1x1", "aspp br3x3", "aspp br7x7"][i]);
            sgemm(co, n, bc, &self.conv1.w[i * bc..], ci, &y.data, n, &mut out, n, true);
            pf.lap("aspp conv1 part");
        }
        let mut t = Tensor::from_vec(&[co, h, w], out);
        t.relu_();
        t
    }
}

fn load_deform(w: &mut Weights, p: &str) -> Result<DeformConv2d, WeightError> {
    let off = w.conv(&format!("{p}.atrous_conv.offset_conv"), 1, 0, 1, true)?;
    let modu = w.conv(&format!("{p}.atrous_conv.modulator_conv"), 1, 0, 1, true)?;
    let (shape, rw) = w.take(&format!("{p}.atrous_conv.regular_conv.weight"))?;
    let rb = w.vec(&format!("{p}.atrous_conv.regular_conv.bias"))?;
    let k = shape[2];
    let mut wcat = off.w;
    wcat.extend_from_slice(&modu.w);
    let mut bcat = off.b.unwrap();
    bcat.extend_from_slice(&modu.b.unwrap());
    let offset_mod = Conv2d::new(wcat, Some(bcat), 3 * k * k, shape[1], k, 1, k / 2, 1);
    Ok(DeformConv2d::new(offset_mod, rw, rb, shape[0], shape[1], k))
}

struct DecBlk {
    conv_in: Conv2d,
    att: AsppDeformable,
    conv_out: Conv2d,
}

impl DecBlk {
    fn load(w: &mut Weights, p: &str) -> Result<Self, WeightError> {
        Ok(DecBlk {
            conv_in: w.conv(&format!("{p}.conv_in"), 1, 1, 1, true)?,
            att: AsppDeformable::load(w, &format!("{p}.dec_att"))?,
            conv_out: w.conv(&format!("{p}.conv_out"), 1, 1, 1, true)?,
        })
    }

    fn forward(&self, x: &Tensor) -> Tensor {
        let mut pf = Prof::detail();
        let mut y = self.conv_in.forward(x);
        y.relu_();
        pf.lap("conv_in");
        let y = self.att.forward(&y);
        pf.lap("att");
        let o = self.conv_out.forward(&y);
        pf.lap("conv_out");
        o
    }
}

struct SimpleConvs {
    conv1: Conv2d,
    conv_out: Conv2d,
}

impl SimpleConvs {
    fn load(w: &mut Weights, p: &str) -> Result<Self, WeightError> {
        Ok(SimpleConvs {
            conv1: w.conv(&format!("{p}.conv1"), 1, 1, 1, true)?,
            conv_out: w.conv(&format!("{p}.conv_out"), 1, 1, 1, true)?,
        })
    }

    fn forward(&self, x: &Tensor) -> Tensor {
        self.conv_out.forward(&self.conv1.forward(x))
    }
}

struct Gdt {
    conv: Conv2d,
    attn: Conv2d,
}

impl Gdt {
    fn load(w: &mut Weights, i: usize) -> Result<Self, WeightError> {
        Ok(Gdt {
            conv: w.conv(&format!("decoder.gdt_convs_{i}.0"), 1, 1, 1, true)?,
            attn: w.conv(&format!("decoder.gdt_convs_attn_{i}.0"), 1, 0, 1, true)?,
        })
    }

    fn gate_(&self, p: &mut Tensor) {
        let mut g = self.conv.forward(p);
        g.relu_();
        let mut a = self.attn.forward(&g);
        a.sigmoid_();
        p.mul_spatial_(&a);
    }
}

pub struct BiRefNet {
    bb: Swin,
    squeeze: DecBlk,
    ipt: [SimpleConvs; 4], // ipt_blk5..ipt_blk2
    /// ipt_blk1 with its conv_out fused into the matching slice of conv_out1 (64 -> 1)
    ipt1: Ipt1,
    dec: [DecBlk; 4], // decoder_block4..decoder_block1
    lat: [Conv2d; 3], // lateral_block4..lateral_block2
    gdt: [Gdt; 3],    // 4..2
    /// conv_out1 restricted to the 96 decoder channels (1x1, 96 -> 1)
    out_dec: Conv2d,
}

/// Intermediate activations recorded for verification against the reference.
pub type Trace = HashMap<&'static str, Tensor>;

impl BiRefNet {
    pub fn load(w: &mut Weights) -> Result<Self, WeightError> {
        let bb = Swin::load(w)?;
        let squeeze = DecBlk::load(w, "squeeze_module.0")?;
        let ipt = [
            SimpleConvs::load(w, "decoder.ipt_blk5")?,
            SimpleConvs::load(w, "decoder.ipt_blk4")?,
            SimpleConvs::load(w, "decoder.ipt_blk3")?,
            SimpleConvs::load(w, "decoder.ipt_blk2")?,
        ];
        let ipt1 = SimpleConvs::load(w, "decoder.ipt_blk1")?;
        let dec = [
            DecBlk::load(w, "decoder.decoder_block4")?,
            DecBlk::load(w, "decoder.decoder_block3")?,
            DecBlk::load(w, "decoder.decoder_block2")?,
            DecBlk::load(w, "decoder.decoder_block1")?,
        ];
        let lat = [
            w.conv("decoder.lateral_block4.conv", 1, 0, 1, true)?,
            w.conv("decoder.lateral_block3.conv", 1, 0, 1, true)?,
            w.conv("decoder.lateral_block2.conv", 1, 0, 1, true)?,
        ];
        let gdt = [Gdt::load(w, 4)?, Gdt::load(w, 3)?, Gdt::load(w, 2)?];
        let out1 = w.conv("decoder.conv_out1.0", 1, 0, 1, true)?;
        let nd = dec[3].conv_out.cout; // 96
        let ni = ipt1.conv_out.cout; // 24
        assert_eq!(out1.cin, nd + ni);
        let out_dec = Conv2d::new(out1.w[..nd].to_vec(), out1.b.clone(), 1, nd, 1, 1, 0, 1);
        // conv_out1[nd..] (1x1, 24->1) after ipt_blk1.conv_out (3x3, 64->24) == one 3x3 conv 64->1
        let wi = &out1.w[nd..];
        let co = &ipt1.conv_out;
        let per = co.cin * co.k * co.k;
        let mut fw = vec![0.0; per];
        let mut fb = 0.0;
        for (o, &a) in wi.iter().enumerate() {
            for (f, &v) in fw.iter_mut().zip(&co.w[o * per..(o + 1) * per]) {
                *f += a * v;
            }
            fb += a * co.b.as_ref().unwrap()[o];
        }
        let ipt1 = Ipt1::new(&ipt1.conv1, fw, fb);
        Ok(BiRefNet { bb, squeeze, ipt, ipt1, dec, lat, gdt, out_dec })
    }

    /// `x`: normalised `[3, S, S]` image (S divisible by 32). Returns logits `[1, S, S]`.
    pub fn forward(&self, x: &Tensor, mut trace: Option<&mut Trace>) -> Tensor {
        let (_, h, w) = x.chw();
        let mut rec = |k: &'static str, t: &Tensor| {
            if let Some(tr) = trace.as_deref_mut() {
                tr.insert(k, t.clone());
            }
        };
        // ---- encoder: full and half resolution passes, concatenated
        let mut prof = Prof::new();
        let full = self.bb.forward(x);
        prof.lap("backbone full");
        let half_in = interp_bilinear_ac(x, h / 2, w / 2);
        let half = self.bb.forward(&half_in);
        prof.lap("backbone half");
        drop(half_in);
        for (i, t) in full.iter().enumerate() {
            rec(["bb0", "bb1", "bb2", "bb3"][i], t);
        }
        let feats: Vec<Tensor> = full
            .iter()
            .zip(&half)
            .map(|(f, hf)| {
                let (_, fh, fw) = f.chw();
                cat_channels(&[f, &interp_bilinear_ac(hf, fh, fw)])
            })
            .collect();
        drop(full);
        drop(half);
        let [x1, x2, x3, x4]: [Tensor; 4] = feats.try_into().unwrap();
        let (_, h4, w4) = x4.chw();
        let x4 = cat_channels(&[
            &interp_bilinear_ac(&x1, h4, w4),
            &interp_bilinear_ac(&x2, h4, w4),
            &interp_bilinear_ac(&x3, h4, w4),
            &x4,
        ]);
        prof.lap("concat");
        let x4 = self.squeeze.forward(&x4);
        prof.lap("squeeze");
        rec("squeeze", &x4);

        // ---- decoder
        let lat_in = [&x3, &x2, &x1];
        let mut p = x4;
        for stage in 0..4 {
            let (_, ph, pw) = p.chw();
            let patches = image_to_patches(x, ph, pw);
            let ipt = self.ipt[stage].forward(&patches);
            let inp = cat_channels(&[&p, &ipt]);
            prof.lap("ipt");
            let mut d = self.dec[stage].forward(&inp);
            prof.lap(["dec4", "dec3", "dec2", "dec1"][stage]);
            if stage == 3 {
                rec("dec1", &d);
                p = d;
                break;
            }
            if stage == 0 {
                rec("d4", &d);
                rec("ipt5", &ipt);
            }
            self.gdt[stage].gate_(&mut d);
            rec(["p4", "p3", "p2"][stage], &d);
            let skip = lat_in[stage];
            let (_, sh, sw) = skip.chw();
            let mut up = interp_bilinear_ac(&d, sh, sw);
            up.add_(&self.lat[stage].forward(skip));
            p = up;
        }
        // conv_out1 is 1x1 and bilinear resizing is linear, so project to 1 channel first
        let mut pf = Prof::detail();
        let z = self.out_dec.forward(&p);
        let mut logits = interp_bilinear_ac(&z, h, w);
        pf.lap("out_dec+interp");
        self.ipt1.add_to(x, &mut logits.data);
        pf.lap("ipt1");
        prof.lap("out");
        rec("logits", &logits);
        logits
    }
}

/// Splits the image into `ph x pw` patches stacked along channels, in the order of
/// the weights' `get_patches_batch`: columns first, then rows, channels innermost,
/// i.e. `c (hg h) (wg w) -> (wg hg c) h w`.
fn image_to_patches(x: &Tensor, ph: usize, pw: usize) -> Tensor {
    let (c, h, w) = x.chw();
    let (gh, gw) = (h / ph, w / pw);
    let mut out = pool::zeros(c * gh * gw * ph * pw);
    out.par_chunks_mut(ph * pw).enumerate().for_each(|(oc, dst)| {
        let ci = oc % c;
        let patch = oc / c;
        let wg = patch / gh;
        let hg = patch % gh;
        let plane = &x.data[ci * h * w..(ci + 1) * h * w];
        for y in 0..ph {
            let src = &plane[(hg * ph + y) * w + wg * pw..][..pw];
            dst[y * pw..(y + 1) * pw].copy_from_slice(src);
        }
    });
    Tensor::from_vec(&[c * gh * gw, ph, pw], out)
}

/// Full-resolution image branch: `ipt_blk1.conv1` (3x3, 3 -> 64) followed by the
/// fused `conv_out` + `conv_out1` slice (3x3, 64 -> 1). Both are linear, so away
/// from the 1-pixel border they compose into a single 5x5 conv 3 -> 1; the border
/// ring (where the intermediate map is zero-padded) is evaluated exactly.
struct Ipt1 {
    /// first conv `[64, 3, 3, 3]` and bias
    w1: Vec<f32>,
    b1: Vec<f32>,
    /// fused second conv `[64, 3, 3]` and bias
    w2: Vec<f32>,
    b2: f32,
    /// composed `[3, 5, 5]` kernel and bias
    w5: Vec<f32>,
    b5: f32,
}

impl Ipt1 {
    fn new(c1: &Conv2d, w2: Vec<f32>, b2: f32) -> Self {
        let (m, cin) = (c1.cout, c1.cin);
        assert_eq!((c1.k, w2.len()), (3, m * 9));
        let b1 = c1.b.clone().unwrap();
        let mut w5 = vec![0.0; cin * 25];
        let mut b5 = b2;
        for mi in 0..m {
            for k2 in 0..9 {
                let a = w2[mi * 9 + k2];
                b5 += a * b1[mi];
                let (y2, x2) = (k2 / 3, k2 % 3);
                for c in 0..cin {
                    for k1 in 0..9 {
                        let (y1, x1) = (k1 / 3, k1 % 3);
                        w5[c * 25 + (y1 + y2) * 5 + x1 + x2] += a * c1.w[(mi * cin + c) * 9 + k1];
                    }
                }
            }
        }
        Ipt1 { w1: c1.w.clone(), b1, w2, b2, w5, b5 }
    }

    /// Adds the branch output to `out` (`[h*w]`).
    fn add_to(&self, x: &Tensor, out: &mut [f32]) {
        let (c, h, w) = x.chw();
        let plane = |ci: usize| &x.data[ci * h * w..(ci + 1) * h * w];
        out.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
            let mut acc = vec![self.b5; w];
            for ci in 0..c {
                let p = plane(ci);
                for dy in 0..5 {
                    let iy = y as isize + dy as isize - 2;
                    if iy < 0 || iy >= h as isize {
                        continue;
                    }
                    let src = &p[iy as usize * w..(iy as usize + 1) * w];
                    for dx in 0..5 {
                        let wt = self.w5[ci * 25 + dy * 5 + dx];
                        let off = dx as isize - 2;
                        let lo = (-off).max(0) as usize;
                        let hi = (w as isize - off).min(w as isize) as usize;
                        let s0 = (lo as isize + off) as usize;
                        for (a, v) in acc[lo..hi].iter_mut().zip(&src[s0..s0 + (hi - lo)]) {
                            *a += wt * v;
                        }
                    }
                }
            }
            let border_row = y == 0 || y == h - 1;
            for (xx, (o, a)) in row.iter_mut().zip(&acc).enumerate() {
                *o += if border_row || xx == 0 || xx == w - 1 { self.exact(x, y, xx) } else { *a };
            }
        });
    }

    fn exact(&self, x: &Tensor, y: usize, xx: usize) -> f32 {
        let (c, h, w) = x.chw();
        let m = self.b1.len();
        let at = |ci: usize, yy: isize, xi: isize| -> f32 {
            if yy < 0 || xi < 0 || yy >= h as isize || xi >= w as isize { 0.0 } else { x.data[ci * h * w + yy as usize * w + xi as usize] }
        };
        let mut s = self.b2;
        for k2 in 0..9 {
            let qy = y as isize + (k2 / 3) as isize - 1;
            let qx = xx as isize + (k2 % 3) as isize - 1;
            if qy < 0 || qx < 0 || qy >= h as isize || qx >= w as isize {
                continue;
            }
            for mi in 0..m {
                let mut v = self.b1[mi];
                for ci in 0..c {
                    for k1 in 0..9 {
                        v += self.w1[(mi * c + ci) * 9 + k1] * at(ci, qy + (k1 / 3) as isize - 1, qx + (k1 % 3) as isize - 1);
                    }
                }
                s += self.w2[mi * 9 + k2] * v;
            }
        }
        s
    }
}
