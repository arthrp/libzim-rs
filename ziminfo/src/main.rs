use libzim_rs::parse_zim;
use std::env;
use std::io::{self, Write};
use std::process::ExitCode;

fn print_usage(program: &str) -> ! {
    let _ = writeln!(io::stderr(), "Usage: {program} <zim-file>");
    std::process::exit(2);
}

fn main() -> ExitCode {
    let program = env::args().next().unwrap_or_else(|| "ziminfo".to_string());
    let Some(path) = env::args().nth(1) else {
        print_usage(&program);
    };

    let zim_file = match parse_zim(&path) {
        Ok(z) => z,
        Err(e) => {
            let _ = writeln!(io::stderr(), "Error: {e}");
            return ExitCode::FAILURE;
        }
    };

    println!(
        "Version: {}.{}",
        zim_file.header.major_version, zim_file.header.minor_version
    );
    println!("Articles: {}", zim_file.header.article_count);
    println!("Clusters: {}", zim_file.header.cluster_count);
    println!("Main page: {}", zim_file.header.main_page);

    println!("MIME types:");
    for m in &zim_file.mime_types {
        println!("  {m}");
    }

    println!("Metadata:");
    for key in zim_file.metadata_keys() {
        match zim_file.get_metadata_str(&key) {
            Some(val) => println!("  {key}: {val}"),
            None => {
                if let Some(bytes) = zim_file.get_metadata(&key) {
                    println!("  {key}: <binary, {} bytes>", bytes.len());
                } else {
                    println!("  {key}: <no content>");
                }
            }
        }
    }

    if let Some(name) = zim_file.get_metadata_str("Name") {
        println!("Name: {name}");
    }

    ExitCode::SUCCESS
}
