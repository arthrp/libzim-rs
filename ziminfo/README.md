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

To include metadata output, pass `-m` before or after the ZIM file path:

```bash
ziminfo -m path/to/file.zim
ziminfo path/to/file.zim -m
```

## Output

By default, prints ZIM version, article and cluster counts, main page index, and MIME types.

With `-m`, also prints all metadata entries and the archive name.
