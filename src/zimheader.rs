use std::io::Read;

pub const ZIM_MAGIC_NUMBER: u32 = 0x044d495a;
pub const HEADER_SIZE: usize = 80;

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

impl ZimHeader {
    pub fn parse_header(reader: &mut impl Read) -> Result<ZimHeader, String> {
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