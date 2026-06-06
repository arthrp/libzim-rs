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

    for c in &zim_file.cluster_pointers {
        println!("pointer: {}", c);
    }

    println!("cached clusters: {}", zim_file.cached_cluster_count());

    for dirent in &zim_file.dirents {
        println!("dirent: {:?}", dirent);

        if !dirent.is_article() {
            continue;
        }

        let mime = zim_file
            .get_mime_type(dirent.mime_type)
            .unwrap_or("unknown");
        let content = zim_file.get_content(dirent);

        // match content {
        //     Some(bytes) => {
        //         println!("  mime: {}, size: {} bytes", mime, bytes.len());
        //         if let Ok(text) = std::str::from_utf8(&bytes) {
        //             let preview: String = text.chars().take(120).collect();
        //             println!("  preview: {}", preview);
        //         }
        //     }
        //     None => println!("  no content available"),
        // }
    }

    println!("cached clusters aftewards: {}", zim_file.cached_cluster_count());
}
