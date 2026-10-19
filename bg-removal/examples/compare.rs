//! Dev check: run the network on a dumped reference input and compare activations.
use bg_removal::birefnet::{BiRefNet, Trace};
use bg_removal::tensor::Tensor;
use bg_removal::weights::Weights;
use std::time::Instant;

fn load_bin(dir: &str, file: &str) -> Vec<f32> {
    let b = std::fs::read(format!("{dir}/{file}")).unwrap();
    b.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect()
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let (wpath, dir) = (&args[1], &args[2]);
    let t = Instant::now();
    let bytes = std::fs::read(wpath).unwrap();
    let mut w = Weights::from_bytes(&bytes).unwrap();
    let net = BiRefNet::load(&mut w).unwrap();
    println!("loaded in {:?}; unused weights: {:?}", t.elapsed(), w.remaining());
    let x = Tensor::from_vec(&[3, 1024, 1024], load_bin(dir, "input.bin"));
    let mut tr = Trace::new();
    let runs: usize = std::env::var("RUNS").ok().and_then(|v| v.parse().ok()).unwrap_or(1);
    for _ in 0..runs {
        let t = Instant::now();
        tr.clear();
        net.forward(&x, Some(&mut tr));
        println!("forward {:?}", t.elapsed());
    }
    let pairs = [
        ("bb0", "bb_norm0_LayerNormalization_output_0.bin", true),
        ("bb1", "bb_norm1_LayerNormalization_output_0.bin", true),
        ("bb2", "bb_norm2_LayerNormalization_output_0.bin", true),
        ("bb3", "bb_norm3_LayerNormalization_output_0.bin", true),
        ("squeeze", "squeeze_module_squeeze_module.0_conv_out_Conv_output_0.bin", false),
        ("ipt5", "decoder_ipt_blk5_conv_out_Conv_output_0.bin", false),
        ("d4", "decoder_decoder_block4_conv_out_Conv_output_0.bin", false),
        ("p4", "decoder_Mul_output_0.bin", false),
        ("p3", "decoder_Mul_1_output_0.bin", false),
        ("p2", "decoder_Mul_2_output_0.bin", false),
        ("dec1", "decoder_decoder_block1_conv_out_Conv_output_0.bin", false),
        ("logits", "output_image.bin", false),
    ];
    for (k, f, tokens) in pairs {
        let Some(mine) = tr.get(k) else { continue };
        let mine = if tokens { mine.chw_to_tokens() } else { mine.clone() };
        let r = load_bin(dir, f);
        assert_eq!(r.len(), mine.len(), "{k}");
        let (mut maxd, mut sd, mut sr) = (0f32, 0f64, 0f64);
        for (a, b) in mine.data.iter().zip(&r) {
            maxd = maxd.max((a - b).abs());
            sd += ((a - b) as f64).powi(2);
            sr += (*b as f64).powi(2);
        }
        println!("{k:8} max|d|={maxd:.5} rel_rms={:.6}", (sd / sr).sqrt());
    }
    let lg = &tr["logits"];
    let r = load_bin(dir, "output_image.bin");
    let s = |v: f32| 1.0 / (1.0 + (-v).exp());
    let maxa = lg.data.iter().zip(&r).map(|(a, b)| (s(*a) - s(*b)).abs()).fold(0f32, f32::max);
    let mean = lg.data.iter().zip(&r).map(|(a, b)| (s(*a) - s(*b)).abs() as f64).sum::<f64>() / r.len() as f64;
    println!("alpha: max|d|={maxa:.5} mean|d|={mean:.6}");
}
