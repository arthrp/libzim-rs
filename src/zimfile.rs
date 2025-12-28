use std::io::{Read, Seek, SeekFrom};
use crate::zimheader::{ZimHeader};

#[derive(Debug)]
pub struct ZimFile {
    pub header: ZimHeader,
    pub mime_types: Vec<String>,
    pub cluster_pointers: Vec<u64>,
}

impl ZimFile {
    pub fn parse_bytes(reader: &mut (impl Read + Seek)) -> Result<Self, String> {
        let header = ZimHeader::parse_header(reader)?;
        let mime_types = ZimFile::parse_mime_types(reader, &header)?;
        let cluster_pointers = ZimFile::parse_cluster_pointers(reader, &header)?;

        Ok(ZimFile { header, mime_types, cluster_pointers })
    }

    fn parse_cluster_pointers(reader: &mut (impl Read + Seek), header: &ZimHeader) -> Result<Vec<u64>, String> {
        reader.seek(SeekFrom::Start(header.cluster_ptr_pos)).map_err(|e| e.to_string())?;
        
        let mut pointers = Vec::with_capacity(header.cluster_count as usize);
        let mut buffer = [0u8; 8];

        for _ in 0..header.cluster_count {
             reader.read_exact(&mut buffer).map_err(|e| e.to_string())?;
             pointers.push(u64::from_le_bytes(buffer));
        }
        
        Ok(pointers)
    }

    fn parse_mime_types(reader: &mut (impl Read + Seek), header: &ZimHeader) -> Result<Vec<String>, String> {
        let mut end_pos = header.path_ptr_pos;
        if header.title_idx_pos > 0 {
            end_pos = std::cmp::min(end_pos, header.title_idx_pos);
        }
        end_pos = std::cmp::min(end_pos, header.cluster_ptr_pos);
        
        let start_pos = header.mime_list_pos;
        if end_pos <= start_pos {
            return Err("Invalid mime list position".to_string());
        }
        
        let size = (end_pos - start_pos) as usize;
        if size > 1024 {
            // TODO: log warning
        }
        
        reader.seek(SeekFrom::Start(start_pos)).map_err(|e| e.to_string())?;
        let mut buffer = vec![0u8; size];
        reader.read_exact(&mut buffer).map_err(|e| e.to_string())?;
        
        let mut mime_types = Vec::new();
        let mut start = 0;
        while start < buffer.len() {
            if buffer[start] == 0 {
                break;
            }
            match buffer[start..].iter().position(|&c| c == 0) {
                Some(len) => {
                    let s = String::from_utf8(buffer[start..start+len].to_vec())
                        .map_err(|e| format!("Invalid UTF-8 in mime type: {}", e))?;
                    mime_types.push(s);
                    start += len + 1;
                },
                None => return Err("Mime list not null terminated".to_string()),
            }
        }
        
        Ok(mime_types)
    }


}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use crate::zimheader::{HEADER_SIZE, ZIM_MAGIC_NUMBER};

    #[test]
    fn test_parse_bytes_less_than_80_bytes() {
        let data = vec![0u8; 79];
        let mut reader = Cursor::new(data);
        let result = ZimFile::parse_bytes(&mut reader);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "failed to fill whole buffer");
    }

    #[test]
    fn test_parse_mime_types() {
        let mut data = vec![0u8; HEADER_SIZE]; // Header
        // Magic number
        let magic = ZIM_MAGIC_NUMBER.to_le_bytes();
        data[0..4].copy_from_slice(&magic);
        
        // Set pointers
        // mime_list_pos at 80
        let mime_list_pos = 80_u64.to_le_bytes();
        data[56..64].copy_from_slice(&mime_list_pos);
        
        // path_ptr_pos at 100 (so 20 bytes for mime types)
        let path_ptr_pos = 100_u64.to_le_bytes();
        data[32..40].copy_from_slice(&path_ptr_pos);
        
        // cluster_ptr_pos at 120
        let cluster_ptr_pos = 120_u64.to_le_bytes();
        data[48..56].copy_from_slice(&cluster_ptr_pos);
        
        // Add mime types: "text/html\0image/png\0"
        let mime_data = b"text/html\0image/png\0";
        data.extend_from_slice(mime_data);
        
        let mut reader = Cursor::new(data);
        let result = ZimFile::parse_bytes(&mut reader);
        assert!(result.is_ok(), "Parse failed: {:?}", result.err());
        let zim = result.unwrap();
        assert_eq!(zim.mime_types.len(), 2);
        assert_eq!(zim.mime_types[0], "text/html");
        assert_eq!(zim.mime_types[1], "image/png");
    }

    #[test]
    fn test_parse_cluster_pointers() {
        let mut data = vec![0u8; HEADER_SIZE];
        
        let magic = ZIM_MAGIC_NUMBER.to_le_bytes();
        data[0..4].copy_from_slice(&magic);
        
        let cluster_count = 2_u32.to_le_bytes();
        data[28..32].copy_from_slice(&cluster_count);
        
        // Pointers
        // mime_list_pos at 80
        let mime_list_pos = 80_u64.to_le_bytes();
        data[56..64].copy_from_slice(&mime_list_pos);
        
        // path_ptr_pos at 90 (10 bytes mime types)
        let path_ptr_pos = 90_u64.to_le_bytes();
        data[32..40].copy_from_slice(&path_ptr_pos);

        // cluster_ptr_pos at 100 (10 bytes path ptrs - dummy)
        let cluster_ptr_pos = 100_u64.to_le_bytes();
        data[48..56].copy_from_slice(&cluster_ptr_pos);
        
        // Data construction
        // 80: Mime types (dummy, 10 bytes)
        data.extend(std::iter::repeat(0).take(10));
        
        // 90: Path pointers (dummy, 10 bytes)
        data.extend(std::iter::repeat(0).take(10));
        
        // 100: Cluster pointers (2 * 8 = 16 bytes)
        // Cluster 0 offset: 1000
        let c0 = 1000_u64.to_le_bytes();
        data.extend_from_slice(&c0);
        
        // Cluster 1 offset: 2000
        let c1 = 2000_u64.to_le_bytes();
        data.extend_from_slice(&c1);
        
        let mut reader = Cursor::new(data);
        let zim = ZimFile::parse_bytes(&mut reader).expect("Parse failed");
        
        assert_eq!(zim.header.cluster_count, 2);
        assert_eq!(zim.cluster_pointers.len(), 2);
        assert_eq!(zim.cluster_pointers[0], 1000);
        assert_eq!(zim.cluster_pointers[1], 2000);
    }
}
