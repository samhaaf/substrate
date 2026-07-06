//! `BackendProvisioner` — auto-provisioning of the llama.cpp inference backend.
//!
//! Substrate v2 does not assume a pre-installed `llama-server` binary. Instead,
//! the provisioner manages the full lifecycle of the backend binary:
//!
//! 1. **Platform detection** — map the current host to a llama.cpp release flavor
//!    (Metal on Apple Silicon, CUDA on NVIDIA Linux, AVX2 CPU builds, etc.).
//! 2. **Release fetching** — resolve the desired llama.cpp build (a specific tag
//!    like `b4935`, or `latest` via the GitHub API), pick the asset matching this
//!    platform, download the zip, and extract it into the data dir.
//! 3. **Lifecycle** — spawn / stop the `llama-server` process with the correct
//!    args (model, port, slots, context size, GPU layers).
//!
//! The extracted binaries are cached under `{data_dir}/backends/llama-{version}/`
//! so a subsequent boot with the same version is a no-op disk check.
//!
//! ## Why provision rather than require?
//!
//! A pre-installed binary is fragile: it may be built for the wrong ISA (no AVX2),
//! the wrong accelerator (CPU-only when CUDA is present), or a llama.cpp version
//! whose `/completion` SSE schema has drifted. Pinning the exact upstream release
//! the substrate node was tested against removes that entire class of "works on my
//! machine" failures.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::Deserialize;
use tokio::sync::broadcast;
use substrate_gc::{EntryKind, GcService};
use substrate_types::{LifecycleEvent, Result, SubstrateError};

use crate::process::{LlamaProcess, LlamaProcessConfig};

// ---------------------------------------------------------------------------
// Platform detection
// ---------------------------------------------------------------------------

/// The inference platform of the current host.
///
/// Each variant maps to a distinct llama.cpp pre-built release flavor. Detection
/// is a compile-time `cfg!` of `target_arch` / `target_os`, refined at runtime
/// where the OS alone is insufficient (e.g. CUDA presence on Linux).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    /// Apple Silicon (M-series). Uses the Metal build.
    MacOsArm64,
    /// Intel Mac. Uses the x64 macOS build (CPU / no Metal accel of note).
    MacOsX64,
    /// Linux x86_64 with an NVIDIA CUDA toolkit/driver present.
    LinuxCuda,
    /// Linux x86_64, CPU-only (AVX2 build).
    LinuxCpu,
    /// Windows x86_64, CPU-only (AVX2 build). Best-effort support.
    WindowsCpu,
}

impl Platform {
    /// Detect the platform of the host this process is running on.
    ///
    /// On Linux we probe for CUDA at runtime (driver/library/`nvidia-smi`) to
    /// distinguish a GPU box from a CPU-only one — the same OS/arch supports both.
    pub fn detect() -> Result<Self> {
        if cfg!(target_os = "macos") {
            if cfg!(target_arch = "aarch64") {
                Ok(Platform::MacOsArm64)
            } else if cfg!(target_arch = "x86_64") {
                Ok(Platform::MacOsX64)
            } else {
                Err(SubstrateError::Config(
                    "unsupported macOS architecture for llama.cpp provisioning".into(),
                ))
            }
        } else if cfg!(target_os = "linux") {
            if cfg!(target_arch = "x86_64") {
                if Self::cuda_present() {
                    Ok(Platform::LinuxCuda)
                } else {
                    Ok(Platform::LinuxCpu)
                }
            } else {
                Err(SubstrateError::Config(
                    "unsupported Linux architecture for llama.cpp provisioning (only x86_64)".into(),
                ))
            }
        } else if cfg!(target_os = "windows") {
            if cfg!(target_arch = "x86_64") {
                Ok(Platform::WindowsCpu)
            } else {
                Err(SubstrateError::Config(
                    "unsupported Windows architecture for llama.cpp provisioning".into(),
                ))
            }
        } else {
            Err(SubstrateError::Config(
                "unsupported operating system for llama.cpp provisioning".into(),
            ))
        }
    }

    /// Heuristic CUDA detection on Linux: a driver library or `nvidia-smi` exists.
    ///
    /// This is intentionally conservative — a false negative just selects the CPU
    /// build (still correct, merely slower), while a false positive would download
    /// a CUDA build that fails to load. We therefore require concrete evidence.
    fn cuda_present() -> bool {
        // Driver runtime library installed by the NVIDIA driver package.
        let lib_paths = [
            "/usr/lib/x86_64-linux-gnu/libcuda.so.1",
            "/usr/lib/x86_64-linux-gnu/libcuda.so",
            "/usr/lib64/libcuda.so.1",
            "/usr/lib64/libcuda.so",
            "/usr/local/cuda/lib64/libcudart.so",
        ];
        if lib_paths.iter().any(|p| Path::new(p).exists()) {
            return true;
        }
        // The management interface device node, present when a GPU + driver exist.
        if Path::new("/dev/nvidiactl").exists() {
            return true;
        }
        // Fall back to the presence of the management CLI on PATH.
        std::process::Command::new("nvidia-smi")
            .arg("--query-gpu=name")
            .arg("--format=csv,noheader")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    /// The substring that identifies this platform's asset in a llama.cpp release.
    ///
    /// llama.cpp release assets are named like:
    /// - `llama-b4935-bin-macos-arm64.zip`
    /// - `llama-b4935-bin-macos-x64.zip`
    /// - `llama-b4935-bin-ubuntu-x64.zip`        (CPU)
    /// - `llama-b4935-bin-ubuntu-cuda-x64.zip`   (CUDA, when published)
    /// - `llama-b4935-bin-win-avx2-x64.zip`
    ///
    /// We match on a discriminating substring rather than the full name because
    /// the build tag (`bXXXX`) is interpolated and the exact suffix ordering has
    /// shifted across upstream releases.
    pub fn asset_matcher(self) -> AssetMatcher {
        match self {
            Platform::MacOsArm64 => AssetMatcher {
                require_all: &["macos", "arm64"],
                exclude_any: &[],
            },
            Platform::MacOsX64 => AssetMatcher {
                require_all: &["macos", "x64"],
                exclude_any: &["arm64"],
            },
            Platform::LinuxCuda => AssetMatcher {
                // Prefer an explicit cuda ubuntu build.
                require_all: &["ubuntu", "cuda", "x64"],
                exclude_any: &[],
            },
            Platform::LinuxCpu => AssetMatcher {
                require_all: &["ubuntu", "x64"],
                exclude_any: &["cuda", "vulkan", "sycl", "hip"],
            },
            Platform::WindowsCpu => AssetMatcher {
                require_all: &["win", "x64"],
                exclude_any: &["cuda", "vulkan", "sycl", "hip", "arm64"],
            },
        }
    }

    /// The binary file name to look for inside the extracted archive.
    pub fn binary_name(self) -> &'static str {
        match self {
            Platform::WindowsCpu => "llama-server.exe",
            _ => "llama-server",
        }
    }
}

/// Predicate over a release-asset file name, used to pick the right download.
#[derive(Debug, Clone, Copy)]
pub struct AssetMatcher {
    /// All of these substrings must be present (case-insensitive).
    require_all: &'static [&'static str],
    /// If any of these substrings is present, the asset is rejected.
    exclude_any: &'static [&'static str],
}

impl AssetMatcher {
    /// True if `name` satisfies the matcher.
    ///
    /// Accepts both `.zip` and `.tar.gz` archives — llama.cpp shifted macOS
    /// releases from `.zip` to `.tar.gz` around build b9859.
    pub fn matches(&self, name: &str) -> bool {
        let lower = name.to_ascii_lowercase();
        if !lower.ends_with(".zip") && !lower.ends_with(".tar.gz") {
            return false;
        }
        if self.exclude_any.iter().any(|x| lower.contains(x)) {
            return false;
        }
        self.require_all.iter().all(|r| lower.contains(r))
    }
}

// ---------------------------------------------------------------------------
// GitHub release model (subset of the API response)
// ---------------------------------------------------------------------------

const LLAMA_REPO: &str = "ggml-org/llama.cpp";

#[derive(Debug, Deserialize)]
struct GithubRelease {
    tag_name: String,
    assets: Vec<GithubAsset>,
}

#[derive(Debug, Deserialize)]
struct GithubAsset {
    name: String,
    browser_download_url: String,
}

// ---------------------------------------------------------------------------
// BackendProvisioner
// ---------------------------------------------------------------------------

/// Provisions and runs the llama.cpp inference backend binary.
///
/// Stateless aside from a `reqwest::Client`; all persistent state lives on disk
/// under the caller-provided data dir. Safe to construct cheaply per operation.
pub struct BackendProvisioner {
    http: reqwest::Client,
    platform: Platform,
    gc: Arc<GcService>,
    /// Optional broadcast channel for lifecycle events.
    event_tx: Option<broadcast::Sender<LifecycleEvent>>,
}

impl BackendProvisioner {
    /// Construct a provisioner, detecting the host platform.
    pub fn new(gc: Arc<GcService>) -> Result<Self> {
        Ok(Self {
            http: reqwest::Client::builder()
                .user_agent("substrate-inference/0.1 (+https://github.com/ggml-org/llama.cpp)")
                .build()
                .map_err(|e| SubstrateError::Engine(format!("failed to build HTTP client: {e}")))?,
            platform: Platform::detect()?,
            gc,
            event_tx: None,
        })
    }

    /// Construct a provisioner for an explicit platform (testing / cross-provision).
    pub fn for_platform(platform: Platform, gc: Arc<GcService>) -> Result<Self> {
        Ok(Self {
            http: reqwest::Client::builder()
                .user_agent("substrate-inference/0.1 (+https://github.com/ggml-org/llama.cpp)")
                .build()
                .map_err(|e| SubstrateError::Engine(format!("failed to build HTTP client: {e}")))?,
            platform,
            gc,
            event_tx: None,
        })
    }

    /// Attach a lifecycle event broadcast channel to this provisioner.
    ///
    /// Once set, emits `BackendInstalling` / `BackendInstalled` during `ensure_binary`,
    /// and `BackendStarting` / `BackendReady` / `BackendStopping` / `BackendStopped`
    /// during `start` / `stop`.
    pub fn with_events(mut self, tx: broadcast::Sender<LifecycleEvent>) -> Self {
        self.event_tx = Some(tx);
        self
    }

    /// Emit a lifecycle event, silently dropping it when there are no subscribers.
    fn emit(&self, event: LifecycleEvent) {
        if let Some(tx) = &self.event_tx {
            let _ = tx.send(event);
        }
    }

    /// The detected (or configured) platform.
    pub fn platform(&self) -> Platform {
        self.platform
    }

    /// Ensure the `llama-server` binary for `version` exists locally, downloading
    /// it if necessary, and return the path to the binary.
    ///
    /// - `data_dir`: substrate data root. Binaries cache under
    ///   `{data_dir}/backends/llama-{version}/`.
    /// - `version`: `"latest"` (resolved via the GitHub API) or a concrete build
    ///   tag like `"b4935"`.
    ///
    /// If the binary already exists in the cache, this is a fast filesystem check
    /// and performs no network I/O (unless `version == "latest"`, which always
    /// resolves the current tag first so a moving `latest` picks up new releases).
    pub async fn ensure_binary(&self, data_dir: &Path, version: &str) -> Result<PathBuf> {
        // Resolve "latest" to a concrete tag so the on-disk cache key is stable.
        let resolved_version = if version == "latest" {
            self.resolve_latest_tag().await?
        } else {
            version.to_string()
        };

        let install_dir = data_dir
            .join("backends")
            .join(format!("llama-{resolved_version}"));
        let binary_name = self.platform.binary_name();

        // Fast path: already provisioned.
        if let Some(existing) = Self::find_binary(&install_dir, binary_name) {
            tracing::debug!(
                version = %resolved_version,
                path = %existing.display(),
                "llama-server already provisioned"
            );
            return Ok(existing);
        }

        tracing::info!(
            version = %resolved_version,
            platform = ?self.platform,
            "provisioning llama-server"
        );

        let platform_name = format!("{:?}", self.platform);
        self.emit(LifecycleEvent::BackendInstalling {
            version: resolved_version.clone(),
            platform: platform_name,
        });

        // Resolve the release and pick the asset for this platform.
        let release = self.fetch_release(&resolved_version).await?;
        let matcher = self.platform.asset_matcher();
        let asset = release
            .assets
            .iter()
            .find(|a| matcher.matches(&a.name))
            .ok_or_else(|| {
                let available: Vec<&str> = release.assets.iter().map(|a| a.name.as_str()).collect();
                SubstrateError::Config(format!(
                    "no llama.cpp release asset matched platform {:?} in release {} (available: {:?})",
                    self.platform, resolved_version, available
                ))
            })?;

        tracing::info!(asset = %asset.name, "downloading llama.cpp release asset");

        // Download into a temp file inside the install dir's parent, then extract.
        std::fs::create_dir_all(&install_dir).map_err(SubstrateError::Io)?;
        let archive_bytes = self.download(&asset.browser_download_url).await?;
        if asset.name.ends_with(".tar.gz") {
            Self::extract_tar_gz(&archive_bytes, &install_dir)?;
        } else {
            Self::extract_zip(&archive_bytes, &install_dir)?;
        }

        // Locate the binary inside the freshly-extracted tree.
        let binary = Self::find_binary(&install_dir, binary_name).ok_or_else(|| {
            SubstrateError::Engine(format!(
                "extracted llama.cpp release {} but could not find {} under {}",
                resolved_version,
                binary_name,
                install_dir.display()
            ))
        })?;

        // Ensure the binary is executable (zip on Unix may not preserve the bit).
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Ok(meta) = std::fs::metadata(&binary) {
                let mut perms = meta.permissions();
                perms.set_mode(perms.mode() | 0o755);
                let _ = std::fs::set_permissions(&binary, perms);
            }
        }

        // Register the install dir and binary with GC.
        if let Err(e) = self.gc.register_dir(&install_dir) {
            tracing::warn!("gc register_dir failed for {}: {e}", install_dir.display());
        }
        if let Err(e) = self.gc.register_entry(&binary, EntryKind::File, Some(format!("github:{LLAMA_REPO}@{resolved_version}"))) {
            tracing::warn!("gc register_entry failed for {}: {e}", binary.display());
        }
        // Lock the binary for 30 days — running binaries must never be evicted.
        let binary_str = binary.to_string_lossy().into_owned();
        if let Err(e) = self.gc.lock(&binary_str, 86400 * 30) {
            tracing::warn!("gc lock failed for {binary_str}: {e}");
        }

        tracing::info!(path = %binary.display(), "llama-server provisioned");
        self.emit(LifecycleEvent::BackendInstalled {
            version: resolved_version,
            binary_path: binary.display().to_string(),
        });
        Ok(binary)
    }

    /// Spawn the llama-server process with the given config.
    pub async fn start(
        &self,
        binary_path: &Path,
        config: &LlamaProcessConfig,
    ) -> Result<LlamaProcess> {
        let path_str = binary_path.display().to_string();
        self.emit(LifecycleEvent::BackendStarting {
            binary_path: path_str.clone(),
            port: config.port,
        });
        let process = LlamaProcess::start_with_config(binary_path, config).await?;
        self.emit(LifecycleEvent::BackendReady {
            binary_path: path_str,
            port: config.port,
        });
        Ok(process)
    }

    /// Stop a running llama-server process.
    ///
    /// Takes `&mut LlamaProcess` per the provisioner contract; internally issues a
    /// best-effort kill and waits for exit. The process is left in a terminated
    /// state (subsequent `is_alive()` returns false).
    pub async fn stop(&self, process: &mut LlamaProcess) -> Result<()> {
        // We don't have the binary path on LlamaProcess, so emit a generic path.
        self.emit(LifecycleEvent::BackendStopping {
            binary_path: String::from("<unknown>"),
        });
        process.terminate().await?;
        self.emit(LifecycleEvent::BackendStopped);
        Ok(())
    }

    // ── internals ──────────────────────────────────────────────────────────

    /// Resolve `latest` to the concrete release tag via the GitHub API.
    async fn resolve_latest_tag(&self) -> Result<String> {
        let url = format!("https://api.github.com/repos/{LLAMA_REPO}/releases/latest");
        let release: GithubRelease = self
            .http
            .get(&url)
            .header("Accept", "application/vnd.github+json")
            .send()
            .await
            .map_err(|e| SubstrateError::Transport(format!("GET {url} failed: {e}")))?
            .error_for_status()
            .map_err(|e| SubstrateError::Engine(format!("GitHub latest-release lookup failed: {e}")))?
            .json()
            .await
            .map_err(|e| SubstrateError::Engine(format!("parsing latest release JSON failed: {e}")))?;
        Ok(release.tag_name)
    }

    /// Fetch a specific release by tag.
    async fn fetch_release(&self, tag: &str) -> Result<GithubRelease> {
        let url = format!("https://api.github.com/repos/{LLAMA_REPO}/releases/tags/{tag}");
        self.http
            .get(&url)
            .header("Accept", "application/vnd.github+json")
            .send()
            .await
            .map_err(|e| SubstrateError::Transport(format!("GET {url} failed: {e}")))?
            .error_for_status()
            .map_err(|e| {
                SubstrateError::Config(format!("GitHub release tag {tag} lookup failed: {e}"))
            })?
            .json()
            .await
            .map_err(|e| SubstrateError::Engine(format!("parsing release {tag} JSON failed: {e}")))
    }

    /// Download a URL into memory (release zips are tens of MB — fine in RAM).
    async fn download(&self, url: &str) -> Result<Vec<u8>> {
        let bytes = self
            .http
            .get(url)
            .send()
            .await
            .map_err(|e| SubstrateError::Transport(format!("download {url} failed: {e}")))?
            .error_for_status()
            .map_err(|e| SubstrateError::Engine(format!("download {url} returned error: {e}")))?
            .bytes()
            .await
            .map_err(|e| SubstrateError::Transport(format!("reading download body failed: {e}")))?;
        Ok(bytes.to_vec())
    }

    /// Extract a zip archive (in memory) into `dest`, preserving Unix mode bits.
    fn extract_zip(zip_bytes: &[u8], dest: &Path) -> Result<()> {
        let reader = std::io::Cursor::new(zip_bytes);
        let mut archive = zip::ZipArchive::new(reader)
            .map_err(|e| SubstrateError::Engine(format!("opening release zip failed: {e}")))?;

        for i in 0..archive.len() {
            let mut file = archive
                .by_index(i)
                .map_err(|e| SubstrateError::Engine(format!("reading zip entry {i} failed: {e}")))?;

            // Guard against path traversal (`..`, absolute paths) in archive names.
            let out_path = match file.enclosed_name() {
                Some(p) => dest.join(p),
                None => {
                    tracing::warn!(name = file.name(), "skipping unsafe zip entry name");
                    continue;
                }
            };

            if file.is_dir() {
                std::fs::create_dir_all(&out_path).map_err(SubstrateError::Io)?;
                continue;
            }

            if let Some(parent) = out_path.parent() {
                std::fs::create_dir_all(parent).map_err(SubstrateError::Io)?;
            }

            let mut out = std::fs::File::create(&out_path).map_err(SubstrateError::Io)?;
            std::io::copy(&mut file, &mut out).map_err(SubstrateError::Io)?;

            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if let Some(mode) = file.unix_mode() {
                    let _ = std::fs::set_permissions(
                        &out_path,
                        std::fs::Permissions::from_mode(mode),
                    );
                }
            }
        }
        Ok(())
    }

    /// Extract a `.tar.gz` archive (in memory) into `dest`, preserving Unix mode bits.
    fn extract_tar_gz(archive_bytes: &[u8], dest: &Path) -> Result<()> {
        use flate2::read::GzDecoder;
        use tar::Archive;

        let reader = std::io::Cursor::new(archive_bytes);
        let decoder = GzDecoder::new(reader);
        let mut archive = Archive::new(decoder);

        for entry in archive.entries()
            .map_err(|e| SubstrateError::Engine(format!("reading tar.gz entries failed: {e}")))?
        {
            let mut entry = entry
                .map_err(|e| SubstrateError::Engine(format!("reading tar.gz entry failed: {e}")))?;

            // Derive output path before consuming the entry.
            let out_path = {
                let path = entry.path()
                    .map_err(|e| SubstrateError::Engine(format!("tar.gz entry path invalid: {e}")))?;
                // Guard against path traversal.
                if path.is_absolute() || path.components().any(|c| c.as_os_str() == "..") {
                    tracing::warn!(name = ?path, "skipping unsafe tar.gz entry name");
                    continue;
                }
                dest.join(&path)
            };

            let is_dir = entry.header().entry_type().is_dir();
            #[cfg(unix)]
            let mode = entry.header().mode().ok();

            if is_dir {
                std::fs::create_dir_all(&out_path).map_err(SubstrateError::Io)?;
                continue;
            }

            if let Some(parent) = out_path.parent() {
                std::fs::create_dir_all(parent).map_err(SubstrateError::Io)?;
            }

            entry.unpack(&out_path)
                .map_err(|e| SubstrateError::Engine(format!("extracting {:?} failed: {e}", out_path)))?;

            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if let Some(mode) = mode {
                    let _ = std::fs::set_permissions(
                        &out_path,
                        std::fs::Permissions::from_mode(mode),
                    );
                }
            }
        }
        Ok(())
    }

    /// Recursively search `dir` for a file named `binary_name`.
    ///
    /// llama.cpp archives place binaries either at the root or under a `build/bin/`
    /// subdirectory depending on the release; a bounded recursive walk handles both
    /// without hard-coding the layout.
    fn find_binary(dir: &Path, binary_name: &str) -> Option<PathBuf> {
        if !dir.exists() {
            return None;
        }
        let mut stack = vec![dir.to_path_buf()];
        while let Some(current) = stack.pop() {
            let entries = std::fs::read_dir(&current).ok()?;
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else if path.file_name().and_then(|n| n.to_str()) == Some(binary_name) {
                    return Some(path);
                }
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_returns_a_platform_on_supported_hosts() {
        // On any supported CI/dev host this must succeed.
        let p = Platform::detect();
        assert!(p.is_ok(), "platform detection failed: {p:?}");
    }

    #[test]
    fn macos_arm64_matches_arm64_asset() {
        let m = Platform::MacOsArm64.asset_matcher();
        assert!(m.matches("llama-b4935-bin-macos-arm64.zip"));
        assert!(!m.matches("llama-b4935-bin-macos-x64.zip"));
        assert!(!m.matches("llama-b4935-bin-ubuntu-x64.zip"));
    }

    #[test]
    fn macos_x64_excludes_arm64() {
        let m = Platform::MacOsX64.asset_matcher();
        assert!(m.matches("llama-b4935-bin-macos-x64.zip"));
        assert!(!m.matches("llama-b4935-bin-macos-arm64.zip"));
    }

    #[test]
    fn linux_cpu_excludes_cuda() {
        let m = Platform::LinuxCpu.asset_matcher();
        assert!(m.matches("llama-b4935-bin-ubuntu-x64.zip"));
        assert!(!m.matches("llama-b4935-bin-ubuntu-cuda-x64.zip"));
    }

    #[test]
    fn linux_cuda_requires_cuda() {
        let m = Platform::LinuxCuda.asset_matcher();
        assert!(m.matches("llama-b4935-bin-ubuntu-cuda-x64.zip"));
        assert!(!m.matches("llama-b4935-bin-ubuntu-x64.zip"));
    }

    #[test]
    fn tar_gz_assets_are_accepted() {
        let m = Platform::MacOsArm64.asset_matcher();
        // llama.cpp shifted macOS releases from .zip to .tar.gz around b9859.
        assert!(m.matches("llama-b9859-bin-macos-arm64.tar.gz"));
        // Non-archive formats are still rejected.
        assert!(!m.matches("llama-b4935-xcframework.zip"));  // doesn't contain "macos"
        assert!(!m.matches("source.tar.gz"));
    }

    #[test]
    fn binary_name_is_exe_on_windows_only() {
        assert_eq!(Platform::WindowsCpu.binary_name(), "llama-server.exe");
        assert_eq!(Platform::MacOsArm64.binary_name(), "llama-server");
        assert_eq!(Platform::LinuxCuda.binary_name(), "llama-server");
    }
}
