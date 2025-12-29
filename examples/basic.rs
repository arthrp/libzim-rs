use libzim_rs::parse_zim;

fn main() {
    let zim_file = parse_zim("/tmp/euler.zim").unwrap();
    println!("{}.{}", zim_file.header.major_version, zim_file.header.minor_version);
    
    for m in zim_file.mime_types {
        println!("Mime found:{}", m);
    }

    for c in zim_file.cluster_pointers {
        println!("pointer: {}", c)
    }

    for cl in zim_file.clusters {
        println!("cluster: {:?}", cl);
    }
}
