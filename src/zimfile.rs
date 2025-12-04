use std::io::{Read, Seek, SeekFrom};
use std::convert::TryInto;

const ZIM_MAGIC_NUMBER: u32 = 0x044d495a;
const HEADER_SIZE: usize = 80;

#[derive(Debug)]
pub struct ZimHeader {
    pub magic_number: u32,
    pub major_version: u16,
    pub minor_version: u16,
    pub uuid: [u8; 16],
    pub article_count: u32,
    pub cluster_count: u32,
    pub path_ptr_pos: u64,
    pub title_idx_pos: u64,
    pub cluster_ptr_pos: u64,
    pub mime_list_pos: u64,
    pub main_page: u32,
    pub layout_page: u32, //Should always be 0xffffffffff
    pub checksum_pos: u64,
}

#[derive(Debug)]
pub struct ZimFile {
    pub header: ZimHeader,
    pub mime_types: Vec<String>,
}

impl ZimFile {
    pub fn parse_bytes(reader: &mut (impl Read + Seek)) -> Result<Self, String> {
        let header = ZimFile::parse_header(reader)?;
        let mime_types = ZimFile::parse_mime_types(reader, &header)?;

        Ok(ZimFile { header, mime_types })
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

    fn parse_header(reader: &mut impl Read) -> Result<ZimHeader, String> {
        let mut buffer = [0u8; HEADER_SIZE];
        reader.read_exact(&mut buffer).map_err(|e| e.to_string())?;

        let magic_number = u32::from_le_bytes(buffer[0..4].try_into().unwrap());
        if magic_number != ZIM_MAGIC_NUMBER {
            return Err("Invalid magic number".to_string());
        }

        let major_version = u16::from_le_bytes(buffer[4..6].try_into().unwrap());
        let minor_version = u16::from_le_bytes(buffer[6..8].try_into().unwrap());
        
        let mut uuid = [0u8; 16];
        uuid.copy_from_slice(&buffer[8..24]);

        let article_count = u32::from_le_bytes(buffer[24..28].try_into().unwrap());
        let cluster_count = u32::from_le_bytes(buffer[28..32].try_into().unwrap());
        let path_ptr_pos = u64::from_le_bytes(buffer[32..40].try_into().unwrap());
        let title_idx_pos = u64::from_le_bytes(buffer[40..48].try_into().unwrap());
        let cluster_ptr_pos = u64::from_le_bytes(buffer[48..56].try_into().unwrap());
        let mime_list_pos = u64::from_le_bytes(buffer[56..64].try_into().unwrap());
        let main_page = u32::from_le_bytes(buffer[64..68].try_into().unwrap());
        let layout_page = u32::from_le_bytes(buffer[68..72].try_into().unwrap());
        let checksum_pos = u64::from_le_bytes(buffer[72..80].try_into().unwrap());

        let header = ZimHeader {
            magic_number,
            major_version,
            minor_version,
            uuid,
            article_count,
            cluster_count,
            path_ptr_pos,
            title_idx_pos,
            cluster_ptr_pos,
            mime_list_pos,
            main_page,
            layout_page,
            checksum_pos,
        };
        Ok(header)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

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
        // Note: parse_mime_types expects null-terminated strings and stops at empty string (double null or end of buffer)
        // But actually my implementation reads up to size and then iterates.
        // "text/html\0image/png\0" is 10 + 10 = 20 bytes exactly?
        // text/html is 9 chars + 1 null = 10 bytes.
        // image/png is 9 chars + 1 null = 10 bytes.
        // Total 20 bytes.
        let mime_data = b"text/html\0image/png\0";
        data.extend_from_slice(mime_data);
        
        // Ensure data is long enough (though Cursor will just be at end)
        // size = 100 - 80 = 20.
        
        let mut reader = Cursor::new(data);
        let result = ZimFile::parse_bytes(&mut reader);
        assert!(result.is_ok(), "Parse failed: {:?}", result.err());
        let zim = result.unwrap();
        assert_eq!(zim.mime_types.len(), 2);
        assert_eq!(zim.mime_types[0], "text/html");
        assert_eq!(zim.mime_types[1], "image/png");
    }
}
