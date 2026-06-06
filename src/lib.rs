use std::path::Path;
use std::fs::File;

mod cache;
mod cluster;
mod dirent;
mod zimfile;
mod zimheader;

pub use zimfile::*;

pub fn parse_zim(file_path: &str) -> Result<ZimFile, String> {
    let p = Path::new(file_path);
    if !p.exists() { return Err("File doesn't exist!".to_string()); }

    let fr = File::open(p).map_err(|e| e.to_string())?;
    let z = ZimFile::parse_bytes(fr)?;
    Ok(z)
}
