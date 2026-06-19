use libzim_rs::parse_zim;

fn main() {
    let zim_file = parse_zim("/tmp/simple_webpage.zim")
        .unwrap();
    println!(
        "{}.{}",
        zim_file.header.major_version, zim_file.header.minor_version
    );

    for m in &zim_file.mime_types {
        println!("Mime found: {}", m);
    }

    for (i, c) in zim_file.cluster_pointers.iter().enumerate() {
        println!("pointer: {}", c);
        if let Some(n) = zim_file.blob_count(i) {
            println!("  cluster {i}: {n} blobs");
            for b in 0..n {
                if let Some(sz) = zim_file.blob_size(i, b) {
                    println!("    blob {b}: {sz} bytes");
                }
            }
        }
    }

    println!("cached clusters: {}", zim_file.cached_cluster_count());

    println!("metadata:");
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

    println!("Name: {}", zim_file.get_metadata_str("Name").unwrap_or("".to_string()));
}
