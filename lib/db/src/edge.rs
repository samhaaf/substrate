//! Edge bundle build + content-addressed storage (design §7.2, §7.7, M3).
//!
//! `source_hash` = sha256 of the BUILT bundle (not the TS source). The built bundle is
//! stored content-addressed under `[dirs].bundles`; recovery (`db edge sync`) re-pushes
//! the STORED bundle, never a rebuild.

use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use substrate_types::{Result, SubstrateError};

use crate::driver::Bundle;

/// Build a bundle from an edge function source dir and store it content-addressed.
///
/// v1 "builds" by concatenating the source dir's files deterministically (the real
/// Deno/esbuild bundling is a shell-out reserved for when the toolchain lands; the
/// content-addressing + storage contract is what v1 pins). Returns the [`Bundle`] with
/// its `source_hash` (M3) and stored `bundle_ref`.
pub fn build_and_store(edge_src_dir: &Path, bundles_dir: &Path) -> Result<Bundle> {
    let bytes = build_bundle_bytes(edge_src_dir)?;
    let source_hash = sha256_hex(&bytes);
    std::fs::create_dir_all(bundles_dir)
        .map_err(|e| SubstrateError::Db(format!("creating {}: {e}", bundles_dir.display())))?;
    let bundle_ref = format!("{}.bundle", &source_hash);
    let stored = bundles_dir.join(&bundle_ref);
    std::fs::write(&stored, &bytes)
        .map_err(|e| SubstrateError::Db(format!("storing bundle {}: {e}", stored.display())))?;
    Ok(Bundle {
        bytes,
        source_hash,
        bundle_ref: stored.to_string_lossy().into_owned(),
    })
}

/// Load a previously-stored bundle by its `bundle_ref` (disaster recovery, §7.3).
pub fn load_stored(bundle_ref: &str) -> Result<Bundle> {
    let bytes = std::fs::read(bundle_ref)
        .map_err(|e| SubstrateError::Db(format!("reading stored bundle {bundle_ref}: {e}")))?;
    let source_hash = sha256_hex(&bytes);
    Ok(Bundle {
        bytes,
        source_hash,
        bundle_ref: bundle_ref.to_string(),
    })
}

/// Deterministic bundle bytes: sorted file names + contents.
fn build_bundle_bytes(dir: &Path) -> Result<Vec<u8>> {
    let mut files: Vec<PathBuf> = Vec::new();
    collect_files(dir, &mut files)?;
    files.sort();
    let mut out = Vec::new();
    for f in files {
        let rel = f.strip_prefix(dir).unwrap_or(&f);
        out.extend_from_slice(rel.to_string_lossy().as_bytes());
        out.push(0);
        let content = std::fs::read(&f)
            .map_err(|e| SubstrateError::Db(format!("reading {}: {e}", f.display())))?;
        out.extend_from_slice(&content);
        out.push(0);
    }
    Ok(out)
}

fn collect_files(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    if !dir.exists() {
        return Err(SubstrateError::Db(format!(
            "edge source dir does not exist: {}",
            dir.display()
        )));
    }
    for e in std::fs::read_dir(dir)
        .map_err(|e| SubstrateError::Db(format!("reading {}: {e}", dir.display())))?
        .flatten()
    {
        let p = e.path();
        if p.is_dir() {
            collect_files(&p, out)?;
        } else {
            out.push(p);
        }
    }
    Ok(())
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    format!("{:x}", h.finalize())
}
