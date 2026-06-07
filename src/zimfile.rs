use std::fmt;
use std::io::{Read, Seek, SeekFrom};
use std::sync::Mutex;

use crate::cache::ClusterCache;
use crate::cluster::Cluster;
use crate::dirent::{Dirent, DirentData};
use crate::zimheader::ZimHeader;

pub const DEFAULT_CACHE_CAPACITY: usize = 16;

pub trait ReadSeek: Read + Seek + Send {}
impl<T: Read + Seek + Send> ReadSeek for T {}

struct ClusterStore {
    reader: Box<dyn ReadSeek>,
    cache: ClusterCache,
}

impl ClusterStore {
    fn cluster(&mut self, idx: usize, offset: u64) -> Option<&Cluster> {
        if !self.cache.contains(idx) {
            self.reader.seek(SeekFrom::Start(offset)).ok()?;
            let cluster = Cluster::parse(&mut self.reader).ok()?;
            self.cache.put(idx, cluster);
        }
        self.cache.get(idx)
    }
}

pub struct ZimFile {
    pub header: ZimHeader,
    pub mime_types: Vec<String>,
    pub cluster_pointers: Vec<u64>,
    pub dirent_pointers: Vec<u64>,
    pub dirents: Vec<Dirent>,
    store: Mutex<ClusterStore>,
}

impl fmt::Debug for ZimFile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ZimFile")
            .field("header", &self.header)
            .field("mime_types", &self.mime_types)
            .field("cluster_pointers", &self.cluster_pointers)
            .field("dirent_pointers", &self.dirent_pointers)
            .field("dirents", &self.dirents)
            .finish_non_exhaustive()
    }
}

impl ZimFile {
    pub fn parse_bytes<R: Read + Seek + Send + 'static>(reader: R) -> Result<Self, String> {
        Self::parse_bytes_with_cache_capacity(reader, DEFAULT_CACHE_CAPACITY)
    }

    pub fn parse_bytes_with_cache_capacity<R: Read + Seek + Send + 'static>(
        mut reader: R,
        capacity: usize,
    ) -> Result<Self, String> {
        let header = ZimHeader::parse_header(&mut reader)?;
        let mime_types = Self::parse_mime_types(&mut reader, &header)?;
        let cluster_pointers = Self::parse_cluster_pointers(&mut reader, &header)?;
        let dirent_pointers = Self::parse_dirent_pointers(&mut reader, &header)?;
        let dirents = Self::parse_dirents(&mut reader, &dirent_pointers)?;
        let store = Mutex::new(ClusterStore {
            reader: Box::new(reader),
            cache: ClusterCache::new(capacity),
        });

        Ok(ZimFile {
            header,
            mime_types,
            cluster_pointers,
            dirent_pointers,
            dirents,
            store,
        })
    }

    fn parse_dirent_pointers(
        reader: &mut (impl Read + Seek),
        header: &ZimHeader,
    ) -> Result<Vec<u64>, String> {
        reader
            .seek(SeekFrom::Start(header.path_ptr_pos))
            .map_err(|e| e.to_string())?;

        let mut pointers = Vec::with_capacity(header.article_count as usize);
        let mut buffer = [0u8; 8];

        for _ in 0..header.article_count {
            reader.read_exact(&mut buffer).map_err(|e| e.to_string())?;
            pointers.push(u64::from_le_bytes(buffer));
        }

        Ok(pointers)
    }

    fn parse_dirents(
        reader: &mut (impl Read + Seek),
        dirent_pointers: &[u64],
    ) -> Result<Vec<Dirent>, String> {
        let mut dirents = Vec::with_capacity(dirent_pointers.len());
        for &offset in dirent_pointers {
            reader
                .seek(SeekFrom::Start(offset))
                .map_err(|e| e.to_string())?;
            let dirent = Dirent::parse(&mut *reader)?;
            dirents.push(dirent);
        }
        Ok(dirents)
    }

    fn parse_cluster_pointers(
        reader: &mut (impl Read + Seek),
        header: &ZimHeader,
    ) -> Result<Vec<u64>, String> {
        reader
            .seek(SeekFrom::Start(header.cluster_ptr_pos))
            .map_err(|e| e.to_string())?;

        let mut pointers = Vec::with_capacity(header.cluster_count as usize);
        let mut buffer = [0u8; 8];

        for _ in 0..header.cluster_count {
            reader.read_exact(&mut buffer).map_err(|e| e.to_string())?;
            pointers.push(u64::from_le_bytes(buffer));
        }

        Ok(pointers)
    }

    fn parse_mime_types(
        reader: &mut (impl Read + Seek),
        header: &ZimHeader,
    ) -> Result<Vec<String>, String> {
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

        reader
            .seek(SeekFrom::Start(start_pos))
            .map_err(|e| e.to_string())?;
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
                    let s = String::from_utf8(buffer[start..start + len].to_vec())
                        .map_err(|e| format!("Invalid UTF-8 in mime type: {}", e))?;
                    mime_types.push(s);
                    start += len + 1;
                }
                None => return Err("Mime list not null terminated".to_string()),
            }
        }

        Ok(mime_types)
    }

    pub fn get_blob(&self, cluster_number: usize, blob_number: usize) -> Option<Vec<u8>> {
        let offset = *self.cluster_pointers.get(cluster_number)?;
        let mut store = self.store.lock().ok()?;
        let cluster = store.cluster(cluster_number, offset)?;
        cluster.get_blob(blob_number).map(|b| b.to_vec())
    }

    pub fn blob_count(&self, cluster_number: usize) -> Option<usize> {
        let offset = *self.cluster_pointers.get(cluster_number)?;
        let mut store = self.store.lock().ok()?;
        Some(store.cluster(cluster_number, offset)?.blob_count())
    }

    pub fn blob_size(&self, cluster_number: usize, blob_number: usize) -> Option<u64> {
        let offset = *self.cluster_pointers.get(cluster_number)?;
        let mut store = self.store.lock().ok()?;
        store.cluster(cluster_number, offset)?.get_blob_size(blob_number)
    }

    pub fn get_content(&self, dirent: &Dirent) -> Option<Vec<u8>> {
        match dirent.data {
            DirentData::Content {
                cluster_number,
                blob_number,
            } => self.get_blob(cluster_number as usize, blob_number as usize),
            _ => None,
        }
    }

    pub fn get_mime_type(&self, mime_type_index: u16) -> Option<&str> {
        if mime_type_index as usize >= self.mime_types.len() {
            return None;
        }
        Some(&self.mime_types[mime_type_index as usize])
    }

    pub fn cached_cluster_count(&self) -> usize {
        self.store.lock().map(|s| s.cache.len()).unwrap_or(0)
    }

    pub fn metadata_keys(&self) -> Vec<String> {
        self.dirents
            .iter()
            .filter(|d| d.namespace == 'M')
            .map(|d| d.url.clone())
            .collect()
    }

    pub fn get_metadata(&self, name: &str) -> Option<Vec<u8>> {
        let dirent = self.find_metadata_dirent(name)?;
        self.get_content(dirent)
    }

    pub fn get_metadata_str(&self, name: &str) -> Option<String> {
        let bytes = self.get_metadata(name)?;
        let mut s = String::from_utf8(bytes).ok()?;
        if s.ends_with('\0') {
            s.pop();
        }
        Some(s)
    }

    fn find_metadata_dirent(&self, name: &str) -> Option<&Dirent> {
        let mut idx = self
            .dirents
            .iter()
            .position(|d| d.namespace == 'M' && d.url == name)?;

        let mut watchdog = 50;
        loop {
            let dirent = &self.dirents[idx];
            if let DirentData::Redirect { redirect_index } = dirent.data {
                if watchdog == 0 {
                    return None;
                }
                watchdog -= 1;
                idx = redirect_index as usize;
                if idx >= self.dirents.len() {
                    return None;
                }
            } else {
                return Some(dirent);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::zimheader::{HEADER_SIZE, ZIM_MAGIC_NUMBER};
    use std::io::Cursor;

    #[test]
    fn test_parse_bytes_less_than_80_bytes() {
        let data = vec![0u8; 79];
        let result = ZimFile::parse_bytes(Cursor::new(data));
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "failed to fill whole buffer");
    }

    #[test]
    fn test_parse_mime_types() {
        let mut data = vec![0u8; HEADER_SIZE];

        let magic = ZIM_MAGIC_NUMBER.to_le_bytes();
        data[0..4].copy_from_slice(&magic);

        let mime_list_pos = 80_u64.to_le_bytes();
        data[56..64].copy_from_slice(&mime_list_pos);

        let path_ptr_pos = 100_u64.to_le_bytes();
        data[32..40].copy_from_slice(&path_ptr_pos);

        let cluster_ptr_pos = 120_u64.to_le_bytes();
        data[48..56].copy_from_slice(&cluster_ptr_pos);

        let mime_data = b"text/html\0image/png\0";
        data.extend_from_slice(mime_data);

        let result = ZimFile::parse_bytes(Cursor::new(data));
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

        let mime_list_pos = 80_u64.to_le_bytes();
        data[56..64].copy_from_slice(&mime_list_pos);

        let path_ptr_pos = 90_u64.to_le_bytes();
        data[32..40].copy_from_slice(&path_ptr_pos);

        let cluster_ptr_pos = 100_u64.to_le_bytes();
        data[48..56].copy_from_slice(&cluster_ptr_pos);

        data.extend(std::iter::repeat(0).take(10));
        data.extend(std::iter::repeat(0).take(10));

        let current_size = data.len() + 16;
        let c0_offset = current_size as u64;
        let c1_offset = c0_offset + 20;

        data.extend_from_slice(&c0_offset.to_le_bytes());
        data.extend_from_slice(&c1_offset.to_le_bytes());

        data.push(0x01);
        data.extend_from_slice(&8u32.to_le_bytes());
        data.extend_from_slice(&10u32.to_le_bytes());
        data.extend(vec![0xAA, 0xBB]);

        while data.len() < c1_offset as usize {
            data.push(0);
        }

        let mut zstd_payload = Vec::new();
        zstd_payload.extend_from_slice(&16u64.to_le_bytes());
        zstd_payload.extend_from_slice(&18u64.to_le_bytes());
        zstd_payload.extend(vec![0xCC, 0xDD]);
        let zstd_compressed = zstd::stream::encode_all(zstd_payload.as_slice(), 0)
            .expect("Failed to compress test cluster");
        data.push(0x15);
        data.extend_from_slice(&zstd_compressed);

        let zim = ZimFile::parse_bytes(Cursor::new(data)).expect("Parse failed");

        assert_eq!(zim.header.cluster_count, 2);
        assert_eq!(zim.cluster_pointers.len(), 2);
        assert_eq!(zim.cluster_pointers[0], c0_offset);
        assert_eq!(zim.cluster_pointers[1], c1_offset);
        assert_eq!(zim.cached_cluster_count(), 0);

        assert_eq!(zim.get_blob(0, 0), Some(vec![0xAA, 0xBB]));
        assert_eq!(zim.get_blob(1, 0), Some(vec![0xCC, 0xDD]));
        assert_eq!(zim.cached_cluster_count(), 2);
    }

    #[test]
    fn test_parse_dirent_pointers_and_dirents() {
        let mut data = vec![0u8; HEADER_SIZE];

        let magic = ZIM_MAGIC_NUMBER.to_le_bytes();
        data[0..4].copy_from_slice(&magic);

        let article_count = 2_u32.to_le_bytes();
        data[24..28].copy_from_slice(&article_count);

        let mime_list_pos = 80_u64.to_le_bytes();
        data[56..64].copy_from_slice(&mime_list_pos);

        let path_ptr_pos = 90_u64.to_le_bytes();
        data[32..40].copy_from_slice(&path_ptr_pos);

        let cluster_ptr_pos = 120_u64.to_le_bytes();
        data[48..56].copy_from_slice(&cluster_ptr_pos);

        data.extend(std::iter::repeat(0).take(10));

        let d0_ptr = 150_u64;
        let d1_ptr = 200_u64;
        data.extend_from_slice(&d0_ptr.to_le_bytes());
        data.extend_from_slice(&d1_ptr.to_le_bytes());

        while data.len() < 120 {
            data.push(0);
        }

        let cluster_count = 0_u32.to_le_bytes();
        data[28..32].copy_from_slice(&cluster_count);

        while data.len() < d0_ptr as usize {
            data.push(0);
        }
        data.extend_from_slice(&1u16.to_le_bytes());
        data.push(0);
        data.push(b'C');
        data.extend_from_slice(&0u32.to_le_bytes());
        data.extend_from_slice(&0u32.to_le_bytes());
        data.extend_from_slice(&0u32.to_le_bytes());
        data.extend_from_slice(b"u0\0t0\0");

        while data.len() < d1_ptr as usize {
            data.push(0);
        }
        data.extend_from_slice(&1u16.to_le_bytes());
        data.push(0);
        data.push(b'C');
        data.extend_from_slice(&0u32.to_le_bytes());
        data.extend_from_slice(&0u32.to_le_bytes());
        data.extend_from_slice(&1u32.to_le_bytes());
        data.extend_from_slice(b"u1\0t1\0");

        let zim = ZimFile::parse_bytes(Cursor::new(data)).expect("Parse failed");

        assert_eq!(zim.header.article_count, 2);
        assert_eq!(zim.dirent_pointers.len(), 2);
        assert_eq!(zim.dirent_pointers[0], d0_ptr);
        assert_eq!(zim.dirent_pointers[1], d1_ptr);
        assert_eq!(zim.dirents.len(), 2);
        assert_eq!(zim.dirents[0].url, "u0");
        assert_eq!(zim.dirents[1].url, "u1");
    }

    #[test]
    fn test_get_content() {
        let mut data = vec![0u8; HEADER_SIZE];

        let magic = ZIM_MAGIC_NUMBER.to_le_bytes();
        data[0..4].copy_from_slice(&magic);

        let article_count = 1_u32.to_le_bytes();
        data[24..28].copy_from_slice(&article_count);

        let cluster_count = 1_u32.to_le_bytes();
        data[28..32].copy_from_slice(&cluster_count);

        let mime_list_pos = 80_u64.to_le_bytes();
        data[56..64].copy_from_slice(&mime_list_pos);

        let path_ptr_pos = 100_u64.to_le_bytes();
        data[32..40].copy_from_slice(&path_ptr_pos);

        let cluster_ptr_pos = 108_u64.to_le_bytes();
        data[48..56].copy_from_slice(&cluster_ptr_pos);

        data.extend(std::iter::repeat(0).take(20));

        let c0_offset = 116_u64;
        let d0_ptr = 130_u64;
        data.extend_from_slice(&d0_ptr.to_le_bytes());
        data.extend_from_slice(&c0_offset.to_le_bytes());

        while data.len() < c0_offset as usize {
            data.push(0);
        }

        data.push(0x01);
        data.extend_from_slice(&8u32.to_le_bytes());
        data.extend_from_slice(&13u32.to_le_bytes());
        data.extend(b"hello");

        while data.len() < d0_ptr as usize {
            data.push(0);
        }

        data.extend_from_slice(&1u16.to_le_bytes());
        data.push(0);
        data.push(b'C');
        data.extend_from_slice(&0u32.to_le_bytes());
        data.extend_from_slice(&0u32.to_le_bytes());
        data.extend_from_slice(&0u32.to_le_bytes());
        data.extend_from_slice(b"article\0Article\0");

        let zim = ZimFile::parse_bytes(Cursor::new(data)).expect("Parse failed");

        let content = zim.get_content(&zim.dirents[0]).expect("content");
        assert_eq!(content, b"hello");
        assert_eq!(zim.get_mime_type(zim.dirents[0].mime_type), None);
    }

    #[test]
    fn test_metadata() {
        use crate::dirent::REDIRECT_MIME_TYPE;

        let mut data = vec![0u8; HEADER_SIZE];

        let magic = ZIM_MAGIC_NUMBER.to_le_bytes();
        data[0..4].copy_from_slice(&magic);

        let article_count = 3_u32.to_le_bytes();
        data[24..28].copy_from_slice(&article_count);

        let cluster_count = 1_u32.to_le_bytes();
        data[28..32].copy_from_slice(&cluster_count);

        let mime_list_pos = 80_u64.to_le_bytes();
        data[56..64].copy_from_slice(&mime_list_pos);

        let path_ptr_pos = 100_u64.to_le_bytes();
        data[32..40].copy_from_slice(&path_ptr_pos);

        let cluster_ptr_pos = 124_u64.to_le_bytes();
        data[48..56].copy_from_slice(&cluster_ptr_pos);

        data.extend(std::iter::repeat(0).take(20));

        let c0_offset = 132_u64;
        let d0_ptr = 157_u64;
        let d1_ptr = 184_u64;
        let d2_ptr = 213_u64;

        data.extend_from_slice(&d0_ptr.to_le_bytes());
        data.extend_from_slice(&d1_ptr.to_le_bytes());
        data.extend_from_slice(&d2_ptr.to_le_bytes());

        while data.len() < 124 {
            data.push(0);
        }

        data.extend_from_slice(&c0_offset.to_le_bytes());

        while data.len() < c0_offset as usize {
            data.push(0);
        }

        data.push(0x01);
        data.extend_from_slice(&12u32.to_le_bytes());
        data.extend_from_slice(&17u32.to_le_bytes());
        data.extend_from_slice(&24u32.to_le_bytes());
        data.extend_from_slice(b"Kiwix");
        data.extend_from_slice(b"Offline");

        while data.len() < d0_ptr as usize {
            data.push(0);
        }

        // M/Publisher -> cluster 0, blob 0
        data.extend_from_slice(&1u16.to_le_bytes());
        data.push(0);
        data.push(b'M');
        data.extend_from_slice(&0u32.to_le_bytes());
        data.extend_from_slice(&0u32.to_le_bytes());
        data.extend_from_slice(&0u32.to_le_bytes());
        data.extend_from_slice(b"Publisher\0\0");

        while data.len() < d1_ptr as usize {
            data.push(0);
        }

        // M/Description -> cluster 0, blob 1
        data.extend_from_slice(&1u16.to_le_bytes());
        data.push(0);
        data.push(b'M');
        data.extend_from_slice(&0u32.to_le_bytes());
        data.extend_from_slice(&0u32.to_le_bytes());
        data.extend_from_slice(&1u32.to_le_bytes());
        data.extend_from_slice(b"Description\0\0");

        while data.len() < d2_ptr as usize {
            data.push(0);
        }

        // M/Title -> redirect to dirent 0 (Publisher)
        data.extend_from_slice(&REDIRECT_MIME_TYPE.to_le_bytes());
        data.push(0);
        data.push(b'M');
        data.extend_from_slice(&0u32.to_le_bytes());
        data.extend_from_slice(&0u32.to_le_bytes());
        data.extend_from_slice(b"Title\0\0");

        let zim = ZimFile::parse_bytes(Cursor::new(data)).expect("Parse failed");

        let mut keys = zim.metadata_keys();
        keys.sort();
        assert_eq!(keys, vec!["Description", "Publisher", "Title"]);

        assert_eq!(zim.get_metadata_str("Publisher"), Some("Kiwix".to_string()));
        assert_eq!(zim.get_metadata_str("Description"), Some("Offline".to_string()));
        assert_eq!(zim.get_metadata_str("Title"), Some("Kiwix".to_string()));
        assert_eq!(zim.get_metadata_str("Unknown"), None);
        assert_eq!(zim.get_metadata("Publisher"), Some(b"Kiwix".to_vec()));
    }

    #[test]
    fn test_cache_eviction() {
        let mut data = vec![0u8; HEADER_SIZE];

        let magic = ZIM_MAGIC_NUMBER.to_le_bytes();
        data[0..4].copy_from_slice(&magic);

        let cluster_count = 2_u32.to_le_bytes();
        data[28..32].copy_from_slice(&cluster_count);

        let mime_list_pos = 80_u64.to_le_bytes();
        data[56..64].copy_from_slice(&mime_list_pos);

        let path_ptr_pos = 90_u64.to_le_bytes();
        data[32..40].copy_from_slice(&path_ptr_pos);

        let cluster_ptr_pos = 100_u64.to_le_bytes();
        data[48..56].copy_from_slice(&cluster_ptr_pos);

        data.extend(std::iter::repeat(0).take(10));
        data.extend(std::iter::repeat(0).take(10));

        let c0_offset = 116_u64;
        let c1_offset = 140_u64;
        data.extend_from_slice(&c0_offset.to_le_bytes());
        data.extend_from_slice(&c1_offset.to_le_bytes());

        while data.len() < c0_offset as usize {
            data.push(0);
        }

        data.push(0x01);
        data.extend_from_slice(&8u32.to_le_bytes());
        data.extend_from_slice(&13u32.to_le_bytes());
        data.extend(b"first");

        while data.len() < c1_offset as usize {
            data.push(0);
        }

        data.push(0x01);
        data.extend_from_slice(&8u32.to_le_bytes());
        data.extend_from_slice(&14u32.to_le_bytes());
        data.extend(b"second");

        let zim = ZimFile::parse_bytes_with_cache_capacity(Cursor::new(data), 1)
            .expect("Parse failed");

        assert_eq!(zim.get_blob(0, 0), Some(b"first".to_vec()));
        assert_eq!(zim.cached_cluster_count(), 1);

        assert_eq!(zim.get_blob(1, 0), Some(b"second".to_vec()));
        assert_eq!(zim.cached_cluster_count(), 1);

        assert_eq!(zim.get_blob(0, 0), Some(b"first".to_vec()));
        assert_eq!(zim.get_blob(1, 0), Some(b"second".to_vec()));
    }
}
