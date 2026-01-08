# libzim-rs

Rust library to parse [zim](https://wiki.openzim.org/wiki/ZIM_file_format) files.

## Motivation

There already exists a [reference implementation](https://github.com/openzim/libzim) of zim file parser in C++. On the other hand, libzim-rs has the following goals:
* Memory safety - written in Rust without unsafe
* Simplicity - only the latest zim version (6.3) is explicitly supported
* Minimum of 3rd party libs
* Batteries included - you don't need to install anything in your system to use this lib
  
