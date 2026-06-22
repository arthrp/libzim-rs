# ziminfo

CLI utility to print summary information from a [ZIM](https://wiki.openzim.org/wiki/ZIM_file_format) file. Built on [libzim-rs](../libzim-rs).

## Usage

```bash
cargo run --manifest-path ziminfo/Cargo.toml -- path/to/file.zim
```

Or after installing from this directory:

```bash
cargo install --path ziminfo
ziminfo path/to/file.zim
```

## Output

Prints ZIM version, article and cluster counts, main page index, MIME types, and other metadata info.
