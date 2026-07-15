# libzim-rs

This is a monorepo with libzim-rs and related tools. READMEs for the individual tools can be found in their respective root directories.

libzim-rs is a Rust library to parse (and later create) [zim](https://wiki.openzim.org/wiki/ZIM_file_format) files.

## Motivation

There already exists a [reference implementation](https://github.com/openzim/libzim) of zim file parser in C++. On the other hand, libzim-rs has the following goals:
* Memory safety - written in Rust without unsafe
* Simplicity - only the latest zim version (6.3) is explicitly supported
* Minimum of 3rd party dependencies
* Batteries included - you don't need to install anything in your system to use this lib
* Cross-platform

In short, it's not trying to replace libzim but can be useful if you need minimalistic and memory-safe zim parser.

## Features

Compared to the reference [libzim](https://github.com/openzim/libzim) (ZIM 6.3):

- [x] ZIM header parsing
- [x] MIME type list, cluster pointer list, and URL/path pointer list
- [x] Dirent parsing: content, redirect, link-target, and deleted entries
- [x] Blob/content extraction with blob count and size
- [x] Metadata (`M` namespace) access with redirect following
- [x] Thread-safe LRU cluster cache
- [x] Cluster compression (`None`, `Zstd`) - legacy ones (zip, etc) not planned to support
- [x] Checksum verification and integrity checks
- [ ] Fulltext search (Xapian) and title-based suggestions
- [ ] Entry lookup by path/title and ordered iteration (path/title/efficient)
- [ ] Random entry, illustrations/favicon access
- [ ] Split ZIM (multipart) and embedded ZIM (fd + offset) reading
- [ ] ZIM file creation/writing
