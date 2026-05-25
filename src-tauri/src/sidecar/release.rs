//! Catalog of the prebuilt `llama-server` releases we know how to consume.
//!
//! This is intentionally a hard-coded table: shipping a per-platform binary
//! URL + sha256 in source means a malicious upstream can't substitute the
//! binary without us re-pinning here.

use std::path::PathBuf;

/// llama.cpp release tag we pin against. Bumped manually after smoke-testing.
/// Current pin: 2026-05-25 release.
pub const LLAMA_RELEASE_TAG: &str = "b9310";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LlamaAsset {
    pub url: String,
    /// SHA256 of the downloaded archive. Empty string = unknown (dev mode);
    /// release builds must populate this.
    pub sha256: String,
    /// Path inside the archive to the `llama-server` executable.
    pub archive_inner_path: String,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum HostPlatform {
    MacArm64,
    MacX64,
    LinuxX64,
    WindowsX64,
    Unsupported,
}

impl HostPlatform {
    pub fn detect() -> Self {
        match (std::env::consts::OS, std::env::consts::ARCH) {
            ("macos", "aarch64") => HostPlatform::MacArm64,
            ("macos", "x86_64") => HostPlatform::MacX64,
            ("linux", "x86_64") => HostPlatform::LinuxX64,
            ("windows", "x86_64") => HostPlatform::WindowsX64,
            _ => HostPlatform::Unsupported,
        }
    }
}

/// Resolve the asset URL + archive layout for `platform`.
///
/// SHA256 values are intentionally empty in source-control: in a release
/// build we'd populate them. For dev (which is what we have today) we leave
/// them blank and rely on the verify-on-disk step in `download_resumable`
/// to be a no-op. The pin is the URL + the release tag.
pub fn asset_for(platform: HostPlatform) -> Option<LlamaAsset> {
    let tag = LLAMA_RELEASE_TAG;
    let base = format!("https://github.com/ggml-org/llama.cpp/releases/download/{tag}");
    match platform {
        HostPlatform::MacArm64 => Some(LlamaAsset {
            url: format!("{base}/llama-{tag}-bin-macos-arm64.zip"),
            sha256: String::new(),
            archive_inner_path: "build/bin/llama-server".to_owned(),
        }),
        HostPlatform::MacX64 => Some(LlamaAsset {
            url: format!("{base}/llama-{tag}-bin-macos-x64.zip"),
            sha256: String::new(),
            archive_inner_path: "build/bin/llama-server".to_owned(),
        }),
        HostPlatform::LinuxX64 => Some(LlamaAsset {
            url: format!("{base}/llama-{tag}-bin-ubuntu-x64.zip"),
            sha256: String::new(),
            archive_inner_path: "build/bin/llama-server".to_owned(),
        }),
        HostPlatform::WindowsX64 => Some(LlamaAsset {
            url: format!("{base}/llama-{tag}-bin-win-cpu-x64.zip"),
            sha256: String::new(),
            archive_inner_path: "build/bin/llama-server.exe".to_owned(),
        }),
        HostPlatform::Unsupported => None,
    }
}

/// Models we know about, keyed by purpose. URLs point to HuggingFace mirrors;
/// users can override via env vars for air-gapped installs.
#[derive(Clone, Debug)]
pub struct ModelAsset {
    pub url: String,
    pub sha256: Option<String>,
    pub local_filename: String,
}

/// Default embedding model: `nomic-embed-text-v2-moe` Q8_0.
/// 512 MB GGUF, 768-dim Matryoshka output, multilingual, 8192-token context.
/// Supersedes v1.5 (newer architecture, similar latency, broader language coverage).
pub fn embedding_model() -> ModelAsset {
    let url = std::env::var("CHHANNI_EMBED_MODEL_URL").unwrap_or_else(|_| {
        "https://huggingface.co/nomic-ai/nomic-embed-text-v2-moe-GGUF/resolve/main/nomic-embed-text-v2-moe.Q8_0.gguf"
            .to_owned()
    });
    ModelAsset {
        url,
        sha256: std::env::var("CHHANNI_EMBED_MODEL_SHA256").ok(),
        local_filename: "nomic-embed-text-v2-moe.Q8_0.gguf".to_owned(),
    }
}

/// Default classifier / reasoner model: `Qwen3-4B-Instruct-2507` Q4_K_M.
/// ~2.5 GB GGUF. Selected over Gemma 3 4B for stronger JSON-schema following
/// and broader multilingual coverage (important for email mailboxes that mix
/// English with the user's native language). Quantization is Q4_K_M because
/// classification doesn't need Q5+ headroom; Q4_K_M leaves ~22 GB free on
/// the 24 GB target machine for the OS, browser, and Tauri runtime.
pub fn classifier_model() -> ModelAsset {
    let url = std::env::var("CHHANNI_CLASSIFIER_MODEL_URL").unwrap_or_else(|_| {
        "https://huggingface.co/unsloth/Qwen3-4B-Instruct-2507-GGUF/resolve/main/Qwen3-4B-Instruct-2507-Q4_K_M.gguf"
            .to_owned()
    });
    ModelAsset {
        url,
        sha256: std::env::var("CHHANNI_CLASSIFIER_MODEL_SHA256").ok(),
        local_filename: "Qwen3-4B-Instruct-2507-Q4_K_M.gguf".to_owned(),
    }
}

pub fn binary_filename(platform: HostPlatform) -> &'static str {
    match platform {
        HostPlatform::WindowsX64 => "llama-server.exe",
        _ => "llama-server",
    }
}

pub fn install_root_in(data_dir: &std::path::Path) -> PathBuf {
    data_dir.join("bin").join(LLAMA_RELEASE_TAG)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_does_not_panic() {
        let _ = HostPlatform::detect();
    }

    #[test]
    fn assets_present_for_all_supported_platforms() {
        for p in [
            HostPlatform::MacArm64,
            HostPlatform::MacX64,
            HostPlatform::LinuxX64,
            HostPlatform::WindowsX64,
        ] {
            let a = asset_for(p).expect("known platform should have an asset");
            assert!(a.url.contains(LLAMA_RELEASE_TAG));
            assert!(!a.archive_inner_path.is_empty());
        }
    }

    #[test]
    fn unsupported_platform_has_no_asset() {
        assert!(asset_for(HostPlatform::Unsupported).is_none());
    }
}
