//! `bgremove` — command line front end.

use bg_removal::{BgRemover, Options};
use std::path::{Path, PathBuf};
use std::time::Instant;

const MODEL_FILE: &str = "birefnet-lite.safetensors";

const USAGE: &str = "\
bgremove - smart background removal (pure Rust, BiRefNet)

USAGE:
    bgremove [OPTIONS] <IMAGE>...

Writes <name>-nobg.png next to every input (transparent background).

OPTIONS:
    -o, --out <PATH>       Output file (one input) or directory (several inputs)
    -m, --model <FILE>     Weights file (default: birefnet-lite.safetensors next to the exe)
        --mask             Also save the alpha mask as <name>-mask.png
        --bg <RRGGBB>      Flatten onto a solid colour instead of transparency
        --no-refine        Skip edge refinement against the full-resolution photo
        --no-decontaminate Keep original colours in semi-transparent pixels
        --keep-all         Keep every detected object, not only the main subject
    -t, --threads <N>      Worker threads (default: all cores)
    -h, --help             Show this help
";

struct Args {
    inputs: Vec<PathBuf>,
    out: Option<PathBuf>,
    model: Option<PathBuf>,
    mask: bool,
    bg: Option<[u8; 3]>,
    opts: Options,
    threads: Option<usize>,
}

fn parse_args() -> Result<Args, String> {
    let mut a = Args { inputs: vec![], out: None, model: None, mask: false, bg: None, opts: Options::default(), threads: None };
    a.opts.release_memory = false;
    let mut it = std::env::args_os().skip(1);
    while let Some(arg) = it.next() {
        let s = arg.to_string_lossy().to_string();
        let mut val = |name: &str| it.next().ok_or_else(|| format!("{name} needs a value"));
        match s.as_str() {
            "-h" | "--help" => {
                print!("{USAGE}");
                std::process::exit(0);
            }
            "-o" | "--out" => a.out = Some(val(&s)?.into()),
            "-m" | "--model" => a.model = Some(val(&s)?.into()),
            "-t" | "--threads" => a.threads = Some(val(&s)?.to_string_lossy().parse().map_err(|_| "bad --threads")?),
            "--mask" => a.mask = true,
            "--bg" => {
                let v = val(&s)?.to_string_lossy().trim_start_matches('#').to_string();
                let c = u32::from_str_radix(&v, 16).map_err(|_| format!("bad colour {v}"))?;
                if v.len() != 6 {
                    return Err(format!("bad colour {v}, expected RRGGBB"));
                }
                a.bg = Some([(c >> 16) as u8, (c >> 8) as u8, c as u8]);
            }
            "--no-refine" => a.opts.refine_edges = false,
            "--no-decontaminate" => a.opts.decontaminate_colors = false,
            "--keep-all" => a.opts.main_subject_only = false,
            _ if s.starts_with('-') && s.len() > 1 => return Err(format!("unknown option {s}")),
            _ => a.inputs.push(arg.into()),
        }
    }
    if a.inputs.is_empty() {
        return Err("no input images".into());
    }
    Ok(a)
}

fn find_model(explicit: Option<PathBuf>) -> Option<PathBuf> {
    if explicit.is_some() {
        return explicit;
    }
    if let Some(p) = std::env::var_os("BG_REMOVAL_MODEL") {
        return Some(p.into());
    }
    let mut cands = vec![];
    if let Ok(exe) = std::env::current_exe() {
        if let Some(d) = exe.parent() {
            cands.push(d.join(MODEL_FILE));
        }
    }
    cands.push(PathBuf::from(MODEL_FILE));
    cands.into_iter().find(|p| p.exists())
}

fn output_path(args: &Args, input: &Path, suffix: &str) -> PathBuf {
    let stem = input.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "image".into());
    let name = format!("{stem}-{suffix}.png");
    match &args.out {
        Some(o) if args.inputs.len() == 1 && suffix == "nobg" && !o.is_dir() => o.clone(),
        Some(o) if o.is_dir() => o.join(name),
        Some(o) if args.inputs.len() > 1 => {
            let _ = std::fs::create_dir_all(o);
            o.join(name)
        }
        _ => input.with_file_name(name),
    }
}

fn process(remover: &BgRemover, args: &Args, input: &Path) -> Result<PathBuf, String> {
    let mut pf = bg_removal::tensor::Prof::new();
    let img = image::open(input).map_err(|e| e.to_string())?.to_rgba8();
    pf.lap("decode");
    let mut cut = remover.remove_background(&img, &args.opts);
    if args.mask {
        let alpha = image::GrayImage::from_fn(cut.width(), cut.height(), |x, y| image::Luma([cut.get_pixel(x, y)[3]]));
        let out = output_path(args, input, "mask");
        alpha.save(&out).map_err(|e| format!("cannot save {}: {e}", out.display()))?;
    }
    let out = output_path(args, input, "nobg");
    pf.lap("remove_background");
    if let Some(bg) = args.bg {
        let flat = image::RgbImage::from_fn(cut.width(), cut.height(), |x, y| {
            let px = cut.get_pixel(x, y);
            let a = px[3] as f32 / 255.0;
            image::Rgb(std::array::from_fn(|c| (px[c] as f32 * a + bg[c] as f32 * (1.0 - a)).round() as u8))
        });
        flat.save(&out).map_err(|e| format!("cannot save {}: {e}", out.display()))?;
    } else {
        // fully transparent pixels carry no colour; zero them so the PNG compresses well
        for px in cut.pixels_mut() {
            if px[3] == 0 {
                *px = image::Rgba([0, 0, 0, 0]);
            }
        }
        cut.save(&out).map_err(|e| format!("cannot save {}: {e}", out.display()))?;
    }
    pf.lap("encode");
    Ok(out)
}

fn main() {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("error: {e}\n\n{USAGE}");
            std::process::exit(2);
        }
    };
    if let Some(n) = args.threads {
        rayon::ThreadPoolBuilder::new().num_threads(n).build_global().ok();
    }
    let Some(model) = find_model(args.model.clone()) else {
        eprintln!("error: model file {MODEL_FILE} not found (put it next to bgremove or pass --model)");
        std::process::exit(1);
    };
    let t = Instant::now();
    let remover = match BgRemover::from_file(&model) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    };
    eprintln!("model loaded in {:.1} s ({} threads)", t.elapsed().as_secs_f32(), rayon::current_num_threads());
    let mut failed = 0;
    for input in &args.inputs {
        let t = Instant::now();
        match process(&remover, &args, input) {
            Ok(out) => eprintln!("{} -> {} ({:.1} s)", input.display(), out.display(), t.elapsed().as_secs_f32()),
            Err(e) => {
                eprintln!("{}: {e}", input.display());
                failed += 1;
            }
        }
    }
    std::process::exit(if failed > 0 { 1 } else { 0 });
}
