//! CPU support check (from KreoKit Cutout), for the Best model only: Fast (RNNoise) picks
//! its own kernels at run time. The bundled ONNX Runtime 1.22 (pyke's static build) needs x86-64-v2:
//! SSSE3, SSE4.1, SSE4.2 and POPCNT outside its runtime-dispatched AVX/FMA kernels
//! (checked by disassembling the macOS, Windows and Linux libraries, 2026-10-13). On an
//! older CPU it would crash with an illegal instruction, so the app checks first and
//! explains instead. Re-check after upgrading ONNX Runtime (CLAUDE.md, "Inference").

use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct CpuSupport {
    pub supported: bool,
    /// Missing instruction sets, e.g. ["SSE4.2"]; empty when supported.
    pub missing: Vec<&'static str>,
}

/// What this machine's CPU lacks for the Best model (always supported off x86-64).
pub fn check() -> CpuSupport {
    let mut missing = missing_features();
    // Development aid: KREOKIT_DEV_MISSING_CPU=SSE4.2 simulates an unsupported CPU.
    if cfg!(debug_assertions) && std::env::var("KREOKIT_DEV_MISSING_CPU").as_deref() == Ok("SSE4.2")
    {
        missing.push("SSE4.2");
    }
    CpuSupport {
        supported: missing.is_empty(),
        missing,
    }
}

#[cfg(target_arch = "x86_64")]
fn missing_features() -> Vec<&'static str> {
    let mut missing = Vec::new();
    if !std::arch::is_x86_feature_detected!("ssse3") {
        missing.push("SSSE3");
    }
    if !std::arch::is_x86_feature_detected!("sse4.1") {
        missing.push("SSE4.1");
    }
    if !std::arch::is_x86_feature_detected!("sse4.2") {
        missing.push("SSE4.2");
    }
    if !std::arch::is_x86_feature_detected!("popcnt") {
        missing.push("POPCNT");
    }
    missing
}

#[cfg(not(target_arch = "x86_64"))]
fn missing_features() -> Vec<&'static str> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_consistently() {
        let s = check();
        assert_eq!(s.supported, s.missing.is_empty());
    }
}
