use bg_removal::tensor::{sgemm, sgemm_par};
use rayon::prelude::*;
use std::time::Instant;
fn bench(f: &mut dyn FnMut(), flops: f64) -> f64 {
    f();
    let t = Instant::now();
    for _ in 0..5 { f(); }
    let dt = t.elapsed().as_secs_f64() / 5.0;
    flops / dt / 1e9
}
fn main() {
    let th = rayon::current_num_threads();
    for &(m, n, k) in &[(4096usize, 1536usize, 384usize), (4900, 1152, 384), (4096, 384, 1536), (67081, 288, 96), (65536, 384, 96), (65536, 96, 384), (16384, 768, 192), (1225, 2304, 768), (1024, 768, 3072)] {
        let a = vec![1.0f32; m * k];
        let b = vec![1.0f32; k * n];
        let mut c = vec![0.0f32; m * n];
        let fl = 2.0 * (m * n * k) as f64;
        let mine = bench(&mut || sgemm(m, n, k, &a, k, &b, n, &mut c, n, false), fl);
        let internal = bench(&mut || sgemm_par(m, n, k, &a, k, &b, n, &mut c, n, false, gemm::Parallelism::Rayon(th)), fl);
        let mut res = vec![];
        for &mb in &[64usize, 128, 512, 1024] {
            let cp = c.as_mut_ptr() as usize;
            res.push((mb, bench(&mut || {
                (0..m.div_ceil(mb)).into_par_iter().for_each(|i| {
                    let i0 = i * mb; let r = mb.min(m - i0);
                    let cs = unsafe { std::slice::from_raw_parts_mut((cp as *mut f32).add(i0 * n), r * n) };
                    sgemm_par(r, n, k, &a[i0 * k..], k, &b, n, cs, n, false, gemm::Parallelism::None);
                });
            }, fl)));
        }
        println!("{m}x{n}x{k}: mine {mine:.0} internal {internal:.0} rows {:?}", res.iter().map(|(a, b)| format!("{a}:{b:.0}")).collect::<Vec<_>>());
    }
}
