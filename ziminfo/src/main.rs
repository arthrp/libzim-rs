use libzim_rs::parse_zim;
use std::env;
use std::io::{self, Write};
use std::process::ExitCode;

fn print_usage(program: &str) -> ! {
    let _ = writeln!(
        io::stderr(),
        "Usage: {program} [-m] <zim-file>\n       {program} <zim-file> [-m]"
    );
    std::process::exit(2);
}

fn parse_args(args: &[String]) -> Option<(String, bool)> {
    let mut path = None;
    let mut show_metadata = false;

    for arg in args {
        match arg.as_str() {
            "-m" => show_metadata = true,
            // Ignore unknown flags
            other if other.starts_with('-') => return None,
            // There can be only one path
            other => {
                if path.is_some() {
                    return None;
                }
                path = Some(other.to_string());
            }
        }
    }

    path.map(|p| (p, show_metadata))
}

fn main() -> ExitCode {
    let program = env::args().next().unwrap_or_else(|| "ziminfo".to_string());
    let args: Vec<String> = env::args().skip(1).collect();
    let Some((path, show_metadata)) = parse_args(&args) else {
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

    if show_metadata {
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
    }

    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::parse_args;

    #[test]
    fn parse_args_path_only() {
        let args = vec!["file.zim".to_string()];
        assert_eq!(parse_args(&args), Some(("file.zim".to_string(), false)));
    }

    #[test]
    fn parse_args_flag_before_path() {
        let args = vec!["-m".to_string(), "file.zim".to_string()];
        assert_eq!(parse_args(&args), Some(("file.zim".to_string(), true)));
    }

    #[test]
    fn parse_args_flag_after_path() {
        let args = vec!["file.zim".to_string(), "-m".to_string()];
        assert_eq!(parse_args(&args), Some(("file.zim".to_string(), true)));
    }

    #[test]
    fn parse_args_no_path() {
        let args = vec!["-m".to_string()];
        assert_eq!(parse_args(&args), None);
    }

    #[test]
    fn parse_args_unknown_flag() {
        let args = vec!["--metadata".to_string(), "file.zim".to_string()];
        assert_eq!(parse_args(&args), None);
    }

    #[test]
    fn parse_args_extra_path() {
        let args = vec!["file.zim".to_string(), "other.zim".to_string()];
        assert_eq!(parse_args(&args), None);
    }
}
