use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use sha2::{Digest, Sha256};

fn main() {
    build_rnnoise();
    tauri_build::build()
}

/// Compiles Xiph's RNNoise (vendored in `rnnoise/`, BSD-3-Clause) with its v0.2 model
/// compiled in, so the Fast model needs no download in the app. The model's C file (78 MB
/// of weights, too big for the repository) comes from Xiph's server, checked against the
/// SHA-256 in `rnnoise/model_version` as Xiph's own `download_model.sh` does.
fn build_rnnoise() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("rnnoise");
    println!("cargo:rerun-if-changed=rnnoise");
    println!("cargo:rerun-if-env-changed=KREOKIT_RNNOISE_MODEL");
    println!("cargo:rerun-if-env-changed=KREOKIT_RNNOISE_XIPH_URL");
    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    let model_dir = rnnoise_model(&root, &out);

    let target_arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap();
    let msvc = env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc");
    let x86 = target_arch == "x86_64" || target_arch == "x86";

    let base = || {
        let mut build = cc::Build::new();
        build
            .include(root.join("include"))
            .include(root.join("src"))
            .include(model_dir.join("src"))
            // Fast even in dev builds: the denoiser runs over whole recordings.
            .opt_level(2)
            .warnings(false);
        if x86 {
            // Runtime CPU detection: SSE4.1 and AVX2 kernels, picked on the user's CPU.
            build.define("RNN_ENABLE_X86_RTCD", None);
            build.define(
                if msvc {
                    "CPU_INFO_BY_C"
                } else {
                    "CPU_INFO_BY_ASM"
                },
                None,
            );
        }
        build
    };

    let src = root.join("src");
    let mut lib = base();
    for file in [
        "denoise.c",
        "rnn.c",
        "pitch.c",
        "kiss_fft.c",
        "celt_lpc.c",
        "nnet.c",
        "nnet_default.c",
        "parse_lpcnet_weights.c",
        "rnnoise_tables.c",
    ] {
        lib.file(src.join(file));
    }
    lib.file(model_dir.join("src/rnnoise_data.c"));
    if x86 {
        lib.file(src.join("x86/x86_dnn_map.c"))
            .file(src.join("x86/x86cpu.c"));
        let mut sse = base();
        sse.file(src.join("x86/nnet_sse4_1.c"));
        let mut avx = base();
        avx.file(src.join("x86/nnet_avx2.c"));
        if msvc {
            // MSVC doesn't define __SSE4_1__; Xiph's x86_arch_macros.h maps these to it.
            for name in [
                "OPUS_X86_MAY_HAVE_SSE",
                "OPUS_X86_MAY_HAVE_SSE2",
                "OPUS_X86_MAY_HAVE_SSE4_1",
            ] {
                sse.define(name, None);
            }
            // /arch:AVX2 defines __AVX2__.
            avx.flag("/arch:AVX2");
        } else {
            sse.flag("-msse4.1");
            avx.flag("-mavx").flag("-mfma").flag("-mavx2");
        }
        sse.compile("rnnoise_sse4_1");
        avx.compile("rnnoise_avx2");
    }
    lib.compile("rnnoise");
}

/// Unpacks the pinned model into `out/rnnoise-model` (downloading it once) and returns
/// that folder.
fn rnnoise_model(root: &Path, out: &Path) -> PathBuf {
    let hash = fs::read_to_string(root.join("model_version"))
        .expect("rnnoise/model_version")
        .trim()
        .to_string();
    let dir = out.join("rnnoise-model");
    let stamp = dir.join("model_version");
    if fs::read_to_string(&stamp).is_ok_and(|s| s == hash) {
        return dir;
    }

    let name = format!("rnnoise_data-{hash}.tar.gz");
    let tarball = match env::var_os("KREOKIT_RNNOISE_MODEL") {
        Some(path) => PathBuf::from(path),
        None => {
            let path = out.join(&name);
            if !path.is_file() {
                download_model(&name, &path);
            }
            path
        }
    };

    let bytes = fs::read(&tarball).unwrap_or_else(|e| panic!("{}: {e}", tarball.display()));
    let actual = format!("{:x}", Sha256::digest(&bytes));
    assert_eq!(
        actual,
        hash,
        "{} doesn't match rnnoise/model_version",
        tarball.display()
    );

    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let status = Command::new("tar")
        .arg("-xzf")
        .arg(&tarball)
        .arg("-C")
        .arg(&dir)
        .args(["src/rnnoise_data.c", "src/rnnoise_data.h"])
        .status()
        .expect("tar is needed to unpack the RNNoise model");
    assert!(status.success(), "unpacking {} failed", tarball.display());
    fs::write(&stamp, &hash).unwrap();
    dir
}

/// Downloads the model archive `name` to `path`: from Xiph, or from our own mirror (the
/// `models-v1` pre-release on kreokit/voice, the same file unchanged) if Xiph's server
/// fails. Either copy is checked against the SHA-256 afterwards.
/// `KREOKIT_RNNOISE_XIPH_URL` replaces Xiph's base URL (to test the fallback).
fn download_model(name: &str, path: &Path) {
    let xiph = env::var("KREOKIT_RNNOISE_XIPH_URL")
        .unwrap_or_else(|_| "https://media.xiph.org/rnnoise/models".into());
    let sources = [
        format!("{xiph}/{name}"),
        format!("https://github.com/kreokit/voice/releases/download/models-v1/{name}"),
    ];
    let part = path.with_extension("part");
    for url in &sources {
        let ok = Command::new("curl")
            .args(["-fsSL", "--retry", "2", "--connect-timeout", "20", "-o"])
            .arg(&part)
            .arg(url)
            .status()
            .expect("curl is needed to download the RNNoise model")
            .success();
        if ok {
            fs::rename(&part, path).unwrap();
            return;
        }
        println!("cargo:warning=downloading {url} failed; trying the next source");
    }
    let _ = fs::remove_file(&part);
    panic!(
        "downloading the RNNoise model failed from {}; set KREOKIT_RNNOISE_MODEL to a local copy",
        sources.join(" and ")
    );
}
