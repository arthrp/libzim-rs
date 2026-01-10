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
    pub is_extended: bool,
    pub blob_offsets: Vec<u64>,
}

impl Cluster {
    pub fn parse(mut reader: impl Read) -> Result<Self, String> {
        let mut byte = [0u8; 1];
        reader.read_exact(&mut byte).map_err(|e| e.to_string())?;
        
        let compression_byte = byte[0];
        let compression_val = compression_byte & 0x0F;
        let is_extended = (compression_byte & 0x10) != 0;
        
        let compression = match compression_val {
            1 => Compression::None,
            2 => Compression::Zip,
            3 => Compression::Bzip2,
            4 => Compression::Lzma,
            5 => Compression::Zstd,
            _ => return Err(format!("Invalid compression type: {}", compression_val)),
        };

        let mut blob_offsets = Vec::new();

        // TODO: Support decompression for Zstd and Lzma.
        // For now we can only parse the offsets if there is no compression.
        // If compressed, we would need to wrap the reader in a decompressor first
        // because the offsets are at the beginning of the uncompressed data.
        if compression == Compression::None {
             if is_extended {
                let mut buf = [0u8; 8];
                reader.read_exact(&mut buf).map_err(|e| e.to_string())?;
                let first_offset = u64::from_le_bytes(buf);
                blob_offsets.push(first_offset);
                
                let count = first_offset / 8;
                // Basic sanity check to prevent OOM on bad data
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
        }

        Ok(Cluster {
            compression,
            is_extended,
            blob_offsets,
        })
    }

    pub fn count(&self) -> usize {
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn test_parse_uncompressed_cluster_32bit() {
        // Construct a simple uncompressed cluster with 2 blobs
        // Compression: None (1) | Not extended (0x00) -> 0x01
        
        let mut data = Vec::new();
        data.push(0x01); // Compression byte
        
        // Offsets (32-bit)
        // Offset table size: 3 entries (start_0, start_1, end_1) -> 3 * 4 = 12 bytes
        // So first offset = 12
        let off0 = 12u32;
        // Blob 0 size = 10 -> next offset = 22
        let off1 = 22u32;
        // Blob 1 size = 5 -> next offset = 27
        let off2 = 27u32;
        
        data.extend_from_slice(&off0.to_le_bytes());
        data.extend_from_slice(&off1.to_le_bytes());
        data.extend_from_slice(&off2.to_le_bytes());
        
        // Blob data
        data.extend(std::iter::repeat(0xAA).take(10)); // Blob 0
        data.extend(std::iter::repeat(0xBB).take(5));  // Blob 1
        
        let mut reader = Cursor::new(data);
        let cluster = Cluster::parse(&mut reader).expect("Failed to parse cluster");
        
        assert_eq!(cluster.compression, Compression::None);
        assert!(!cluster.is_extended);
        assert_eq!(cluster.blob_offsets.len(), 3);
        assert_eq!(cluster.blob_offsets[0], 12);
        assert_eq!(cluster.blob_offsets[1], 22);
        assert_eq!(cluster.blob_offsets[2], 27);
        
        assert_eq!(cluster.count(), 2);
        assert_eq!(cluster.get_blob_size(0), Some(10));
        assert_eq!(cluster.get_blob_size(1), Some(5));
    }

    #[test]
    fn test_parse_compressed_cluster_info() {
        // Just test that we correctly identify compression type even if we don't parse offsets
        let data = vec![0x15]; // Zstd (5) | Extended (0x10)
        let mut reader = Cursor::new(data);
        let cluster = Cluster::parse(&mut reader).expect("Failed to parse cluster");
        
        assert_eq!(cluster.compression, Compression::Zstd);
        assert!(cluster.is_extended);
        assert!(cluster.blob_offsets.is_empty());
    }
}
