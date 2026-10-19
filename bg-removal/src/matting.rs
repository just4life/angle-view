//! Post-processing of the predicted alpha matte: resizing, edge refinement with a
//! colour guided filter, removal of stray islands and foreground colour
//! estimation (background colour decontamination).
//!
//! Images here are planar `f32` in `[0, 1]`: `rgb` is `[3][h*w]` concatenated.

use rayon::prelude::*;

/// Bilinear resize of a single-channel image (pixel-centre sampling, like PIL / `align_corners=False`).
pub fn resize_plane(src: &[f32], sw: usize, sh: usize, dw: usize, dh: usize) -> Vec<f32> {
    if (sw, sh) == (dw, dh) {
        return src.to_vec();
    }
    let taps = |d: usize, s: usize| -> Vec<(usize, usize, f32)> {
        let scale = s as f32 / d as f32;
        (0..d)
            .map(|i| {
                let x = ((i as f32 + 0.5) * scale - 0.5).max(0.0);
                let x0 = (x.floor() as usize).min(s - 1);
                let x1 = (x0 + 1).min(s - 1);
                (x0, x1, x - x0 as f32)
            })
            .collect()
    };
    let tx = taps(dw, sw);
    let ty = taps(dh, sh);
    let mut out = vec![0.0; dw * dh];
    out.par_chunks_mut(dw).enumerate().for_each(|(y, row)| {
        let (y0, y1, fy) = ty[y];
        let r0 = &src[y0 * sw..(y0 + 1) * sw];
        let r1 = &src[y1 * sw..(y1 + 1) * sw];
        for (v, &(x0, x1, fx)) in row.iter_mut().zip(&tx) {
            let a = r0[x0] + (r0[x1] - r0[x0]) * fx;
            let b = r1[x0] + (r1[x1] - r1[x0]) * fx;
            *v = a + (b - a) * fy;
        }
    });
    out
}

/// Mean over a `(2r+1)^2` window, clipped at the borders (normalised by the true count).
pub fn box_mean(src: &[f32], w: usize, h: usize, r: usize) -> Vec<f32> {
    // horizontal pass
    let mut tmp = vec![0.0; w * h];
    tmp.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
        let s = &src[y * w..(y + 1) * w];
        let mut acc = 0.0f64;
        let mut lo = 0usize;
        let mut hi = 0usize; // window is [lo, hi)
        for (x, o) in row.iter_mut().enumerate() {
            let want_hi = (x + r + 1).min(w);
            let want_lo = x.saturating_sub(r);
            while hi < want_hi {
                acc += s[hi] as f64;
                hi += 1;
            }
            while lo < want_lo {
                acc -= s[lo] as f64;
                lo += 1;
            }
            *o = (acc / (hi - lo) as f64) as f32;
        }
    });
    // vertical pass, processed in column strips for cache friendliness
    let mut out = vec![0.0; w * h];
    const STRIP: usize = 64;
    let strips: Vec<usize> = (0..w).step_by(STRIP).collect();
    let outp = crate::tensor::SendPtr(out.as_mut_ptr());
    strips.par_iter().for_each(|&x0| {
        let outp = outp;
        let x1 = (x0 + STRIP).min(w);
        let nw = x1 - x0;
        let mut acc = vec![0.0f64; nw];
        let (mut lo, mut hi) = (0usize, 0usize);
        for y in 0..h {
            let want_hi = (y + r + 1).min(h);
            let want_lo = y.saturating_sub(r);
            while hi < want_hi {
                for (a, v) in acc.iter_mut().zip(&tmp[hi * w + x0..hi * w + x1]) {
                    *a += *v as f64;
                }
                hi += 1;
            }
            while lo < want_lo {
                for (a, v) in acc.iter_mut().zip(&tmp[lo * w + x0..lo * w + x1]) {
                    *a -= *v as f64;
                }
                lo += 1;
            }
            let inv = 1.0 / (hi - lo) as f64;
            for (i, a) in acc.iter().enumerate() {
                // SAFETY: each strip writes only its own columns.
                unsafe { *outp.0.add(y * w + x0 + i) = (*a * inv) as f32 };
            }
        }
    });
    out
}

/// Averages `f x f` blocks (the last partial block is averaged over what exists).
fn downsample(src: &[f32], w: usize, h: usize, f: usize) -> (Vec<f32>, usize, usize) {
    let (dw, dh) = (w.div_ceil(f), h.div_ceil(f));
    let mut out = vec![0.0; dw * dh];
    out.par_chunks_mut(dw).enumerate().for_each(|(y, row)| {
        for (x, o) in row.iter_mut().enumerate() {
            let mut s = 0.0;
            let mut n = 0;
            for yy in y * f..((y + 1) * f).min(h) {
                for xx in x * f..((x + 1) * f).min(w) {
                    s += src[yy * w + xx];
                    n += 1;
                }
            }
            *o = s / n as f32;
        }
    });
    (out, dw, dh)
}

/// Fast colour guided filter (He et al.) of `p` guided by `rgb`, evaluated on a
/// grid subsampled by `sub`, with radius `r` and regulariser `eps` in full-resolution units.
pub fn guided_filter(rgb: &[f32], p: &[f32], w: usize, h: usize, r: usize, eps: f32, sub: usize) -> Vec<f32> {
    let n = w * h;
    let sub = sub.max(1);
    let (ir, lw, lh) = {
        let (r_, lw, lh) = downsample(&rgb[..n], w, h, sub);
        let (g_, _, _) = downsample(&rgb[n..2 * n], w, h, sub);
        let (b_, _, _) = downsample(&rgb[2 * n..], w, h, sub);
        ([r_, g_, b_], lw, lh)
    };
    let (pl, _, _) = downsample(p, w, h, sub);
    let rr = (r / sub).max(1);
    let m = |v: &[f32]| box_mean(v, lw, lh, rr);
    let mi: Vec<Vec<f32>> = ir.iter().map(|c| m(c)).collect();
    let mp = m(&pl);
    let prod = |a: &[f32], b: &[f32]| -> Vec<f32> { a.par_iter().zip(b.par_iter()).map(|(x, y)| x * y).collect() };
    let mip: Vec<Vec<f32>> = ir.iter().map(|c| m(&prod(c, &pl))).collect();
    let pairs = [(0, 0), (0, 1), (0, 2), (1, 1), (1, 2), (2, 2)];
    let mii: Vec<Vec<f32>> = pairs.iter().map(|&(i, j)| m(&prod(&ir[i], &ir[j]))).collect();
    let ln = lw * lh;
    let mut a = vec![[0.0f32; 3]; ln];
    let mut b = vec![0.0f32; ln];
    a.par_iter_mut().zip(b.par_iter_mut()).enumerate().for_each(|(k, (ak, bk))| {
        let mu = [mi[0][k], mi[1][k], mi[2][k]];
        let cov_ip = [mip[0][k] - mu[0] * mp[k], mip[1][k] - mu[1] * mp[k], mip[2][k] - mu[2] * mp[k]];
        let s = |q: usize, i: usize, j: usize| mii[q][k] - mu[i] * mu[j];
        let (s00, s01, s02, s11, s12, s22) = (s(0, 0, 0) + eps, s(1, 0, 1), s(2, 0, 2), s(3, 1, 1) + eps, s(4, 1, 2), s(5, 2, 2) + eps);
        // inverse of the symmetric 3x3 matrix via cofactors
        let c00 = s11 * s22 - s12 * s12;
        let c01 = s02 * s12 - s01 * s22;
        let c02 = s01 * s12 - s02 * s11;
        let c11 = s00 * s22 - s02 * s02;
        let c12 = s01 * s02 - s00 * s12;
        let c22 = s00 * s11 - s01 * s01;
        let det = s00 * c00 + s01 * c01 + s02 * c02;
        let inv = 1.0 / det;
        let av = [
            (c00 * cov_ip[0] + c01 * cov_ip[1] + c02 * cov_ip[2]) * inv,
            (c01 * cov_ip[0] + c11 * cov_ip[1] + c12 * cov_ip[2]) * inv,
            (c02 * cov_ip[0] + c12 * cov_ip[1] + c22 * cov_ip[2]) * inv,
        ];
        *ak = av;
        *bk = mp[k] - av[0] * mu[0] - av[1] * mu[1] - av[2] * mu[2];
    });
    let ma: Vec<Vec<f32>> = (0..3).map(|c| m(&a.iter().map(|v| v[c]).collect::<Vec<_>>())).collect();
    let mb = m(&b);
    let up = |v: &[f32]| upsample_blocks(v, lw, lh, w, h, sub);
    let (ua, ub) = ((up(&ma[0]), up(&ma[1]), up(&ma[2])), up(&mb));
    let mut q = vec![0.0; n];
    q.par_iter_mut().enumerate().for_each(|(i, v)| {
        *v = ua.0[i] * rgb[i] + ua.1[i] * rgb[n + i] + ua.2[i] * rgb[2 * n + i] + ub[i];
    });
    q
}

/// Bilinear upsampling of a block-averaged grid back to full resolution.
fn upsample_blocks(v: &[f32], lw: usize, lh: usize, w: usize, h: usize, f: usize) -> Vec<f32> {
    if f == 1 {
        return v.to_vec();
    }
    let taps = |d: usize, s: usize| -> Vec<(usize, usize, f32)> {
        (0..d)
            .map(|i| {
                // block k covers [k*f, (k+1)*f), centre (k + 0.5) * f
                let x = ((i as f32 + 0.5) / f as f32 - 0.5).clamp(0.0, (s - 1) as f32);
                let x0 = x.floor() as usize;
                let x1 = (x0 + 1).min(s - 1);
                (x0, x1, x - x0 as f32)
            })
            .collect()
    };
    let tx = taps(w, lw);
    let ty = taps(h, lh);
    let mut out = vec![0.0; w * h];
    out.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
        let (y0, y1, fy) = ty[y];
        for (o, &(x0, x1, fx)) in row.iter_mut().zip(&tx) {
            let a = v[y0 * lw + x0] + (v[y0 * lw + x1] - v[y0 * lw + x0]) * fx;
            let b = v[y1 * lw + x0] + (v[y1 * lw + x1] - v[y1 * lw + x0]) * fx;
            *o = a + (b - a) * fy;
        }
    });
    out
}

/// Refines a soft alpha against the image: the guided-filter result replaces the
/// alpha only inside the uncertain band around the subject's contour.
pub fn refine_alpha(rgb: &[f32], alpha: &[f32], w: usize, h: usize) -> Vec<f32> {
    let scale = (w.max(h) as f32 / 1024.0).max(1.0);
    let r = (3.0 * scale).round() as usize;
    let sub = (scale.floor() as usize).clamp(1, 4);
    let gf = guided_filter(rgb, alpha, w, h, r, 1e-4, sub);
    // uncertain band: not clearly inside nor outside, widened by the filter radius
    let unsure: Vec<f32> = alpha.par_iter().map(|&a| if a > 0.02 && a < 0.98 { 1.0 } else { 0.0 }).collect();
    let band = box_mean(&unsure, w, h, r * 2);
    alpha
        .par_iter()
        .zip(gf.par_iter())
        .zip(band.par_iter())
        .map(|((&a, &g), &bm)| {
            if bm <= 0.0 {
                a
            } else {
                // blend in proportion to how deep inside the band we are
                let t = (bm * 4.0).min(1.0);
                (a + (g.clamp(0.0, 1.0) - a) * t).clamp(0.0, 1.0)
            }
        })
        .collect()
}

/// Keeps the main subject: solid regions (alpha > 0.5) whose area is at least
/// `min_frac` of the largest one, plus soft pixels near them. Works on any
/// resolution; call it on the network-resolution matte for speed.
pub fn keep_main_subject(alpha: &mut [f32], w: usize, h: usize, min_frac: f32) {
    let n = w * h;
    let mut label = vec![u32::MAX; n];
    let mut areas: Vec<usize> = Vec::new();
    let mut stack = Vec::new();
    for s in 0..n {
        if alpha[s] <= 0.5 || label[s] != u32::MAX {
            continue;
        }
        let id = areas.len() as u32;
        let mut area = 0;
        label[s] = id;
        stack.push(s);
        while let Some(p) = stack.pop() {
            area += 1;
            let (y, x) = ((p / w) as isize, (p % w) as isize);
            for dy in -1..=1 {
                for dx in -1..=1 {
                    let (ny, nx) = (y + dy, x + dx);
                    if ny < 0 || nx < 0 || ny >= h as isize || nx >= w as isize {
                        continue;
                    }
                    let q = ny as usize * w + nx as usize;
                    if label[q] == u32::MAX && alpha[q] > 0.5 {
                        label[q] = id;
                        stack.push(q);
                    }
                }
            }
        }
        areas.push(area);
    }
    let Some(&largest) = areas.iter().max() else { return };
    let keep: Vec<bool> = areas.iter().map(|&a| a as f32 >= min_frac * largest as f32).collect();
    let kept: Vec<f32> = label.iter().map(|&l| if l != u32::MAX && keep[l as usize] { 1.0 } else { 0.0 }).collect();
    let radius = (w.max(h) / 40).max(2);
    let near = box_mean(&kept, w, h, radius);
    alpha.par_iter_mut().zip(near.par_iter()).zip(label.par_iter()).for_each(|((a, &nb), &l)| {
        let solid_dropped = l != u32::MAX && !keep[l as usize];
        if solid_dropped || nb <= 0.0 {
            *a = 0.0;
        }
    });
}

/// Fast multi-level foreground estimation (Germer et al. 2020, as in pymatting's
/// `estimate_foreground_ml`), solved with red-black Gauss-Seidel sweeps.
/// Returns the planar foreground colours.
pub fn estimate_foreground(rgb: &[f32], alpha: &[f32], w0: usize, h0: usize) -> Vec<f32> {
    const REG: f32 = 1e-5;
    const GRAD_W: f32 = 1.0;
    let n0 = w0 * h0;
    let levels = (w0.max(h0) as f32).log2().ceil() as usize;
    let mut fb_prev: Vec<f32> = Vec::new(); // [F r,g,b, B r,g,b] planes
    let (mut wp, mut hp) = (0usize, 0usize);
    for lvl in 0..=levels {
        let w = ((w0 as f32).powf(lvl as f32 / levels as f32).round() as usize).max(1);
        let h = ((h0 as f32).powf(lvl as f32 / levels as f32).round() as usize).max(1);
        let n = w * h;
        let xs: Vec<usize> = (0..w).map(|x| x * w0 / w).collect();
        let ys: Vec<usize> = (0..h).map(|y| y * h0 / h).collect();
        let mut img = vec![0.0f32; 3 * n];
        let mut al = vec![0.0f32; n];
        for y in 0..h {
            for x in 0..w {
                let s = ys[y] * w0 + xs[x];
                al[y * w + x] = alpha[s];
                for c in 0..3 {
                    img[c * n + y * w + x] = rgb[c * n0 + s];
                }
            }
        }
        let mut fb = vec![0.0f32; 6 * n];
        if fb_prev.is_empty() {
            for c in 0..3 {
                let mean = img[c * n..(c + 1) * n].iter().sum::<f32>() / n as f32;
                fb[c * n..(c + 1) * n].fill(mean);
                fb[(3 + c) * n..(4 + c) * n].fill(mean);
            }
        } else {
            let pn = wp * hp;
            for y in 0..h {
                for x in 0..w {
                    let s = (y * hp / h) * wp + x * wp / w;
                    for c in 0..6 {
                        fb[c * n + y * w + x] = fb_prev[c * pn + s];
                    }
                }
            }
        }
        let iters = if w <= 32 && h <= 32 { 10 } else { 2 };
        for _ in 0..iters {
            for parity in 0..2 {
                sweep(&img, &al, &mut fb, w, h, parity, REG, GRAD_W);
            }
        }
        fb_prev = fb;
        wp = w;
        hp = h;
    }
    fb_prev.truncate(3 * n0);
    fb_prev
}

#[allow(clippy::too_many_arguments)]
fn sweep(img: &[f32], al: &[f32], fb: &mut [f32], w: usize, h: usize, parity: usize, reg: f32, gw: f32) {
    let n = w * h;
    let fbp = crate::tensor::SendPtr(fb.as_mut_ptr());
    (0..h).into_par_iter().for_each(|y| {
        let fbp = fbp;
        let get = |c: usize, i: usize| unsafe { *fbp.0.add(c * n + i) };
        let x_start = (y + parity) % 2;
        for x in (x_start..w).step_by(2) {
            let i = y * w + x;
            let a0 = al[i];
            let a1 = 1.0 - a0;
            let (mut a00, a01, mut a11) = (a0 * a0, a0 * a1, a1 * a1);
            let mut bf = [a0 * img[i], a0 * img[n + i], a0 * img[2 * n + i]];
            let mut bb = [a1 * img[i], a1 * img[n + i], a1 * img[2 * n + i]];
            let nbrs = [
                y * w + x.saturating_sub(1),
                y * w + (x + 1).min(w - 1),
                y.saturating_sub(1) * w + x,
                (y + 1).min(h - 1) * w + x,
            ];
            for &j in &nbrs {
                let da = reg + gw * (a0 - al[j]).abs();
                a00 += da;
                a11 += da;
                for c in 0..3 {
                    bf[c] += da * get(c, j);
                    bb[c] += da * get(3 + c, j);
                }
            }
            let inv = 1.0 / (a00 * a11 - a01 * a01);
            let (b00, b01, b11) = (inv * a11, -inv * a01, inv * a00);
            for c in 0..3 {
                let f = (b00 * bf[c] + b01 * bb[c]).clamp(0.0, 1.0);
                let b = (b01 * bf[c] + b11 * bb[c]).clamp(0.0, 1.0);
                // SAFETY: pixels of one parity only read neighbours of the other parity,
                // so rows written concurrently never alias a pixel being read or written.
                unsafe {
                    *fbp.0.add(c * n + i) = f;
                    *fbp.0.add((3 + c) * n + i) = b;
                }
            }
        }
    });
}
