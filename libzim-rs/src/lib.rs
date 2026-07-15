use std::fs::File;
use std::path::Path;

mod cache;
mod cluster;
mod dirent;
mod zimfile;
mod zimheader;

pub use cluster::{Cluster, Compression};
pub use zimfile::*;

pub fn parse_zim(file_path: &str) -> Result<ZimFile, String> {
    let p = Path::new(file_path);
    if !p.exists() {
        return Err("File doesn't exist!".to_string());
    }

    let fr = File::open(p).map_err(|e| e.to_string())?;
    let z = ZimFile::parse_bytes(fr)?;
    Ok(z)
}

/// Open a ZIM archive and run the requested integrity checks.
///
/// Returns `false` if opening fails or any selected check fails.
pub fn validate(path: &str, checks: &[IntegrityCheck]) -> bool {
    let Ok(zim) = parse_zim(path) else {
        return false;
    };

    checks.iter().all(|check| zim.check_integrity(*check))
}
