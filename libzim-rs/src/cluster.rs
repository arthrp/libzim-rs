use std::io::Read;

const MAX_BLOBS: u64 = 1_000_000;

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum Compression {
    None = 1,
    Zip = 2,
    Bzip2 = 3,
    Lzma = 4,
    Zstd = 5,
}

#[derive(Debug)]
pub struct Cluster {
    pub compression: Compression,
    pub blob_offsets: Vec<u64>,
    blob_data: Vec<u8>,
}

impl Cluster {
    pub fn parse(mut reader: impl Read) -> Result<Self, String> {
        let mut byte = [0u8; 1];
        reader.read_exact(&mut byte).map_err(|e| e.to_string())?;

        let compression_byte = byte[0];
        let compression_val = compression_byte & 0x0F;
        let is_extended = (compression_byte & 0x10) != 0;

        let compression = match compression_val {
            0 | 1 => Compression::None,
            2 => Compression::Zip,
            3 => Compression::Bzip2,
            4 => Compression::Lzma,
            5 => Compression::Zstd,
            _ => return Err(format!("Invalid compression type: {}", compression_val)),
        };

        match compression {
            Compression::Zip | Compression::Bzip2 | Compression::Lzma => {
                return Err(format!("Unsupported compression type: {:?}", compression));
            }
            Compression::None | Compression::Zstd => {}
        }

        let (blob_offsets, blob_data) = if compression == Compression::Zstd {
            let mut decompressed = Vec::new();
            let mut decoder = zstd::stream::read::Decoder::new(&mut reader)
                .map_err(|e| format!("Failed to create zstd decoder: {}", e))?
                .single_frame();
            decoder
                .read_to_end(&mut decompressed)
                .map_err(|e| format!("Failed to decompress zstd cluster: {}", e))?;
            read_cluster_payload(&mut decompressed.as_slice(), is_extended)?
        } else {
            read_cluster_payload(&mut reader, is_extended)?
        };

        Ok(Cluster {
            compression,
            blob_offsets,
            blob_data,
        })
    }

    pub fn blob_count(&self) -> usize {
        if self.blob_offsets.is_empty() {
            0
        } else {
            self.blob_offsets.len() - 1
        }
    }

    pub fn get_blob_size(&self, index: usize) -> Option<u64> {
        if index + 1 >= self.blob_offsets.len() {
            return None;
        }
        Some(self.blob_offsets[index + 1] - self.blob_offsets[index])
    }

    pub fn get_blob(&self, index: usize) -> Option<&[u8]> {
        if index + 1 >= self.blob_offsets.len() {
            return None;
        }
        let base = self.blob_offsets[0];
        let start = (self.blob_offsets[index] - base) as usize;
        let end = (self.blob_offsets[index + 1] - base) as usize;
        self.blob_data.get(start..end)
    }
}

fn read_cluster_payload(reader: &mut impl Read, is_extended: bool) -> Result<(Vec<u64>, Vec<u8>), String> {
    let blob_offsets = read_blob_offsets(reader, is_extended)?;

    let mut blob_data = Vec::new();
    if !blob_offsets.is_empty() {
        let base = blob_offsets[0];
        let end = *blob_offsets.last().unwrap();
        let data_len = (end - base) as usize;
        blob_data.resize(data_len, 0);
        reader
            .read_exact(&mut blob_data)
            .map_err(|e| format!("Failed to read blob data: {}", e))?;
    }

    Ok((blob_offsets, blob_data))
}

fn read_blob_offsets(reader: &mut impl Read, is_extended: bool) -> Result<Vec<u64>, String> {
    let mut blob_offsets = Vec::new();

    if is_extended {
        let mut buf = [0u8; 8];
        reader.read_exact(&mut buf).map_err(|e| e.to_string())?;
        let first_offset = u64::from_le_bytes(buf);
        blob_offsets.push(first_offset);

        let count = first_offset / 8;
        if count > MAX_BLOBS {
            return Err(format!("Too many blobs in cluster: {}", count));
        }

        for _ in 1..count {
            reader.read_exact(&mut buf).map_err(|e| e.to_string())?;
            let offset = u64::from_le_bytes(buf);
            blob_offsets.push(offset);
        }
    } else {
        let mut buf = [0u8; 4];
        reader.read_exact(&mut buf).map_err(|e| e.to_string())?;
        let first_offset = u32::from_le_bytes(buf) as u64;
        blob_offsets.push(first_offset);

        let count = first_offset / 4;
        if count > MAX_BLOBS {
            return Err(format!("Too many blobs in cluster: {}", count));
        }

        for _ in 1..count {
            reader.read_exact(&mut buf).map_err(|e| e.to_string())?;
            let offset = u32::from_le_bytes(buf) as u64;
            blob_offsets.push(offset);
        }
    }

    Ok(blob_offsets)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn build_uncompressed_cluster_payload() -> Vec<u8> {
        let mut payload = Vec::new();
        let off0 = 12u32;
        let off1 = 22u32;
        let off2 = 27u32;

        payload.extend_from_slice(&off0.to_le_bytes());
        payload.extend_from_slice(&off1.to_le_bytes());
        payload.extend_from_slice(&off2.to_le_bytes());
        payload.extend(std::iter::repeat(0xAA).take(10));
        payload.extend(std::iter::repeat(0xBB).take(5));
        payload
    }

    fn build_extended_cluster_payload() -> Vec<u8> {
        let mut payload = Vec::new();
        let off0 = 24u64;
        let off1 = 34u64;
        let off2 = 39u64;

        payload.extend_from_slice(&off0.to_le_bytes());
        payload.extend_from_slice(&off1.to_le_bytes());
        payload.extend_from_slice(&off2.to_le_bytes());
        payload.extend(std::iter::repeat(0xAA).take(10));
        payload.extend(std::iter::repeat(0xBB).take(5));
        payload
    }

    #[test]
    fn test_parse_uncompressed_cluster_32bit() {
        let mut data = Vec::new();
        data.push(0x01);
        data.extend(build_uncompressed_cluster_payload());

        let mut reader = Cursor::new(data);
        let cluster = Cluster::parse(&mut reader).expect("Failed to parse cluster");

        assert_eq!(cluster.blob_offsets.len(), 3);
        assert_eq!(cluster.compression, Compression::None);
        assert_eq!(cluster.blob_offsets[0], 12);
        assert_eq!(cluster.blob_offsets[1], 22);
        assert_eq!(cluster.blob_offsets[2], 27);

        assert_eq!(cluster.blob_count(), 2);
        assert_eq!(cluster.get_blob_size(0), Some(10));
        assert_eq!(cluster.get_blob_size(1), Some(5));
        assert_eq!(cluster.get_blob(0), Some(&[0xAA; 10][..]));
        assert_eq!(cluster.get_blob(1), Some(&[0xBB; 5][..]));
    }

    #[test]
    fn test_parse_zstd_cluster() {
        let payload = build_uncompressed_cluster_payload();
        let compressed = zstd::stream::encode_all(payload.as_slice(), 0).expect("Failed to compress test cluster");

        let mut data = Vec::new();
        data.push(0x05); // Zstd, not extended
        data.extend_from_slice(&compressed);

        let mut reader = Cursor::new(data);
        let cluster = Cluster::parse(&mut reader).expect("Failed to parse zstd cluster");

        assert_eq!(cluster.compression, Compression::Zstd);
        assert_eq!(cluster.blob_count(), 2);
        assert_eq!(cluster.get_blob(0), Some(&[0xAA; 10][..]));
        assert_eq!(cluster.get_blob(1), Some(&[0xBB; 5][..]));
    }

    #[test]
    fn test_parse_zstd_cluster_with_trailing_data() {
        // A real ZIM stores clusters contiguously, so the bytes following a
        // zstd frame belong to the next cluster. The decoder must stop at the
        // frame boundary rather than trying to read the trailing data as
        // another concatenated frame.
        let payload = build_uncompressed_cluster_payload();
        let compressed = zstd::stream::encode_all(payload.as_slice(), 0).expect("Failed to compress test cluster");

        let mut data = Vec::new();
        data.push(0x05); // Zstd, not extended
        data.extend_from_slice(&compressed);
        // Trailing bytes that are NOT a valid zstd frame (e.g. the next cluster).
        data.push(0x05);
        data.extend_from_slice(&[0xDE, 0xAD, 0xBE, 0xEF]);

        let mut reader = Cursor::new(data);
        let cluster = Cluster::parse(&mut reader).expect("Failed to parse zstd cluster");

        assert_eq!(cluster.blob_count(), 2);
        assert_eq!(cluster.get_blob(0), Some(&[0xAA; 10][..]));
        assert_eq!(cluster.get_blob(1), Some(&[0xBB; 5][..]));
    }

    #[test]
    fn test_parse_compressed_cluster_info() {
        let payload = build_extended_cluster_payload();
        let compressed = zstd::stream::encode_all(payload.as_slice(), 0).expect("Failed to compress test cluster");

        let mut data = Vec::new();
        data.push(0x15); // Zstd (5) | Extended (0x10)
        data.extend_from_slice(&compressed);

        let mut reader = Cursor::new(data);
        let cluster = Cluster::parse(&mut reader).expect("Failed to parse cluster");

        assert_eq!(cluster.blob_count(), 2);
        assert_eq!(cluster.get_blob(0), Some(&[0xAA; 10][..]));
    }

    #[test]
    fn test_unsupported_compression() {
        let data = vec![0x04]; // Lzma
        let mut reader = Cursor::new(data);
        let result = Cluster::parse(&mut reader);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Unsupported compression"));
    }
}
