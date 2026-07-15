use std::fmt;
use std::io::{Read, Seek, SeekFrom};
use std::sync::Mutex;

use md5::{Digest, Md5};

use crate::cache::ClusterCache;
use crate::cluster::{Cluster, Compression};
use crate::dirent::{Dirent, DirentData};
use crate::zimheader::{HEADER_SIZE, ZimHeader};

pub const DEFAULT_CACHE_CAPACITY: usize = 16;

const CHUNK_SIZE: usize = 1024;
const DIRENT_MIN_SIZE: u64 = 11;
const CLUSTER_MIN_SIZE: u64 = 1;
const TITLE_LISTING_V1_PATH: &str = "listing/titleOrdered/v1";

/// Integrity check kinds, matching libzim's `IntegrityCheck` enum order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntegrityCheck {
    /// MD5 of file bytes excluding stored checksum digest
    Checksum,
    /// Path pointer offsets fall within the valid data range
    DirentPtrs,
    /// Dirents are strictly sorted by `{namespace}/{url}`
    DirentOrder,
    /// Title listing indices are in range and sorted by `{namespace}/{title}`
    TitleIndex,
    /// Cluster pointer offsets fall within the valid data range
    ClusterPtrs,
    /// Every cluster can be parsed successfully
    ClustersOffsets,
    /// Article dirent MIME indices are within the MIME list
    DirentMimeTypes,
}

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

    fn parse_cluster_at(&mut self, offset: u64) -> Result<Cluster, String> {
        self.reader
            .seek(SeekFrom::Start(offset))
            .map_err(|e| e.to_string())?;
        Cluster::parse(&mut self.reader)
    }

    fn read_bytes_at(&mut self, offset: u64, size: usize) -> Result<Vec<u8>, String> {
        self.reader
            .seek(SeekFrom::Start(offset))
            .map_err(|e| e.to_string())?;
        let mut buffer = vec![0u8; size];
        self.reader
            .read_exact(&mut buffer)
            .map_err(|e| e.to_string())?;
        Ok(buffer)
    }
}

pub struct ZimFile {
    pub header: ZimHeader,
    pub mime_types: Vec<String>,
    pub cluster_pointers: Vec<u64>,
    pub dirent_pointers: Vec<u64>,
    pub dirents: Vec<Dirent>,
    file_size: u64,
    cluster_store: Mutex<ClusterStore>,
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

        let file_size = reader
            .seek(SeekFrom::End(0))
            .map_err(|e| e.to_string())?;
        if header.has_checksum() && header.get_checksum_pos() + 16 != file_size {
            return Err("Zim file is of bad size or corrupted.".to_string());
        }

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
            file_size,
            cluster_store: store,
        })
    }

    pub fn file_size(&self) -> u64 {
        self.file_size
    }

    pub fn has_checksum(&self) -> bool {
        self.header.has_checksum()
    }

    /// Get checksum as hex string
    pub fn get_checksum(&self) -> Option<String> {
        if !self.header.has_checksum() {
            return None;
        }

        let checksum_pos = self.header.get_checksum_pos();
        let mut store = self.cluster_store.lock().ok()?;
        let stored = store.read_bytes_at(checksum_pos, 16).ok()?;
        Some(stored.iter().map(|byte| format!("{:02x}", byte)).collect())
    }

    pub fn check(&self) -> bool {
        if !self.header.has_checksum() {
            return false;
        }

        let checksum_pos = self.header.get_checksum_pos();
        let mut store = match self.cluster_store.lock() {
            Ok(store) => store,
            Err(_) => return false,
        };

        if store.reader.seek(SeekFrom::Start(0)).is_err() {
            return false;
        }

        let mut hasher = Md5::new();
        let mut remaining = checksum_pos;
        let mut chunk = [0u8; CHUNK_SIZE];

        while remaining > 0 {
            let to_read = std::cmp::min(remaining as usize, CHUNK_SIZE);
            if store.reader.read_exact(&mut chunk[..to_read]).is_err() {
                return false;
            }
            hasher.update(&chunk[..to_read]);
            remaining -= to_read as u64;
        }

        let calculated = hasher.finalize();
        let stored = match store.read_bytes_at(checksum_pos, 16) {
            Ok(bytes) => bytes,
            Err(_) => return false,
        };

        calculated.as_slice() == stored.as_slice()
    }

    pub fn check_integrity(&self, check: IntegrityCheck) -> bool {
        match check {
            IntegrityCheck::Checksum => self.check(),
            IntegrityCheck::DirentPtrs => self.check_dirent_ptrs(),
            IntegrityCheck::DirentOrder => self.check_dirent_order(),
            IntegrityCheck::TitleIndex => self.check_title_index(),
            IntegrityCheck::ClusterPtrs => self.check_cluster_ptrs(),
            IntegrityCheck::ClustersOffsets => self.check_clusters_offsets(),
            IntegrityCheck::DirentMimeTypes => self.check_dirent_mime_types(),
        }
    }

    fn valid_data_end(&self) -> u64 {
        if self.header.has_checksum() {
            self.header.get_checksum_pos()
        } else {
            self.file_size
        }
    }

    fn check_dirent_ptrs(&self) -> bool {
        let end = self.valid_data_end();
        for &offset in &self.dirent_pointers {
            if offset < HEADER_SIZE as u64
                || offset + DIRENT_MIN_SIZE > end
            {
                return false;
            }
        }
        true
    }

    fn check_dirent_order(&self) -> bool {
        let mut prev: Option<&Dirent> = None;
        for dirent in &self.dirents {
            if let Some(prev) = prev {
                if prev.long_path() >= dirent.long_path() {
                    return false;
                }
            }
            prev = Some(dirent);
        }
        true
    }

    fn check_cluster_ptrs(&self) -> bool {
        let end = self.valid_data_end();
        for &offset in &self.cluster_pointers {
            if offset < HEADER_SIZE as u64 || offset + CLUSTER_MIN_SIZE > end {
                return false;
            }
        }
        true
    }

    fn check_clusters_offsets(&self) -> bool {
        let mut store = match self.cluster_store.lock() {
            Ok(store) => store,
            Err(_) => return false,
        };

        for &offset in &self.cluster_pointers {
            if store.parse_cluster_at(offset).is_err() {
                return false;
            }
        }
        true
    }

    fn check_dirent_mime_types(&self) -> bool {
        let mime_count = self.mime_types.len() as u16;
        for dirent in &self.dirents {
            if dirent.is_article() && dirent.mime_type >= mime_count {
                return false;
            }
        }
        true
    }

    fn check_title_index(&self) -> bool {
        let mut ok = true;

        if self.header.has_title_listing_v0() {
            ok &= self.check_title_listing_v0();
        }

        if let Some(indices) = self.read_title_listing_v1_indices() {
            ok &= self.check_title_listing(&indices);
        }

        ok
    }

    fn check_title_listing_v0(&self) -> bool {
        let count = self.header.article_count as usize;
        if count == 0 {
            return true;
        }

        let Some(size) = count.checked_mul(std::mem::size_of::<u32>()) else {
            return false;
        };

        let mut store = match self.cluster_store.lock() {
            Ok(store) => store,
            Err(_) => return false,
        };

        let bytes = match store.read_bytes_at(self.header.title_idx_pos, size) {
            Ok(bytes) => bytes,
            Err(_) => return false,
        };

        let indices = parse_u32_indices(&bytes);
        self.check_title_listing(&indices)
    }

    fn read_title_listing_v1_indices(&self) -> Option<Vec<u32>> {
        let idx = self
            .dirents
            .iter()
            .position(|d| d.namespace == 'X' && d.url == TITLE_LISTING_V1_PATH)?;

        let dirent = &self.dirents[idx];
        let DirentData::Content {
            cluster_number,
            blob_number,
        } = dirent.data
        else {
            return None;
        };

        if self.cluster_compression(cluster_number as usize)? != Compression::None {
            return None;
        }

        let bytes = self.get_blob(cluster_number as usize, blob_number as usize)?;
        Some(parse_u32_indices(&bytes))
    }

    fn check_title_listing(&self, indices: &[u32]) -> bool {
        let article_count = self.header.article_count;
        let mut prev: Option<&Dirent> = None;

        for &idx in indices {
            if idx >= article_count {
                return false;
            }

            let dirent = &self.dirents[idx as usize];
            if let Some(prev) = prev {
                if prev.pseudo_title() > dirent.pseudo_title() {
                    return false;
                }
            }
            prev = Some(dirent);
        }

        true
    }

    fn parse_dirent_pointers(reader: &mut (impl Read + Seek), header: &ZimHeader) -> Result<Vec<u64>, String> {
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

    fn parse_dirents(reader: &mut (impl Read + Seek), dirent_pointers: &[u64]) -> Result<Vec<Dirent>, String> {
        let mut dirents = Vec::with_capacity(dirent_pointers.len());
        for &offset in dirent_pointers {
            reader.seek(SeekFrom::Start(offset)).map_err(|e| e.to_string())?;
            let dirent = Dirent::parse(&mut *reader)?;
            dirents.push(dirent);
        }
        Ok(dirents)
    }

    fn parse_cluster_pointers(reader: &mut (impl Read + Seek), header: &ZimHeader) -> Result<Vec<u64>, String> {
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

    fn parse_mime_types(reader: &mut (impl Read + Seek), header: &ZimHeader) -> Result<Vec<String>, String> {
        let mut end_pos = header.path_ptr_pos;
        if header.has_title_listing_v0() {
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
        let mut store = self.cluster_store.lock().ok()?;
        let cluster = store.cluster(cluster_number, offset)?;
        cluster.get_blob(blob_number).map(|b| b.to_vec())
    }

    pub fn blob_count(&self, cluster_number: usize) -> Option<usize> {
        let offset = *self.cluster_pointers.get(cluster_number)?;
        let mut store = self.cluster_store.lock().ok()?;
        Some(store.cluster(cluster_number, offset)?.blob_count())
    }

    pub fn blob_size(&self, cluster_number: usize, blob_number: usize) -> Option<u64> {
        let offset = *self.cluster_pointers.get(cluster_number)?;
        let mut store = self.cluster_store.lock().ok()?;
        store.cluster(cluster_number, offset)?.get_blob_size(blob_number)
    }

    pub fn cluster_compression(&self, cluster_number: usize) -> Option<Compression> {
        let offset = *self.cluster_pointers.get(cluster_number)?;
        let mut store = self.cluster_store.lock().ok()?;
        Some(store.cluster(cluster_number, offset)?.compression)
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
        self.cluster_store.lock().map(|s| s.cache.len()).unwrap_or(0)
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
        let mut idx = self.dirents.iter().position(|d| d.namespace == 'M' && d.url == name)?;

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

fn parse_u32_indices(bytes: &[u8]) -> Vec<u32> {
    bytes
        .chunks_exact(4)
        .map(|chunk| u32::from_le_bytes(chunk.try_into().unwrap()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::zimheader::{HEADER_SIZE, ZIM_MAGIC_NUMBER};
    use std::io::Cursor;

    fn append_checksum(data: &mut Vec<u8>) {
        let checksum_pos = data.len() as u64;
        data[72..80].copy_from_slice(&checksum_pos.to_le_bytes());
        let mut hasher = Md5::new();
        hasher.update(data.as_slice());
        let digest = hasher.finalize();
        data.extend_from_slice(&digest);
    }

    fn empty_zim_archive_content() -> Vec<u8> {
        let mut data = vec![0u8; HEADER_SIZE];
        data[0..4].copy_from_slice(b"ZIM\x04");
        data[4] = 0x05;

        let pos = 0x51_u64;
        data[32..40].copy_from_slice(&pos.to_le_bytes());
        data[40..48].copy_from_slice(&pos.to_le_bytes());
        data[48..56].copy_from_slice(&pos.to_le_bytes());
        data[56..64].copy_from_slice(&0x50_u64.to_le_bytes());
        data[72..80].copy_from_slice(&pos.to_le_bytes());

        data.push(0);
        data.extend_from_slice(&[
            0x9f, 0x3e, 0xcd, 0x95, 0x46, 0xf6, 0xc5, 0x3b, 0x35, 0xb4, 0xc6, 0xd4, 0xc0, 0x8e, 0xd0, 0x66,
        ]);
        data
    }

    fn pad_to(data: &mut Vec<u8>, offset: u64) {
        while data.len() < offset as usize {
            data.push(0);
        }
    }

    fn ensure_empty_mime(data: &mut Vec<u8>) {
        pad_to(data, 81);
        data[80] = 0;
    }

    fn write_header(data: &mut Vec<u8>) {
        let magic = ZIM_MAGIC_NUMBER.to_le_bytes();
        data[0..4].copy_from_slice(&magic);
        let mime_list_pos = 80_u64.to_le_bytes();
        data[56..64].copy_from_slice(&mime_list_pos);
        data[40..48].copy_from_slice(&u64::MAX.to_le_bytes());
        ensure_empty_mime(data);
    }

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
        data[40..48].copy_from_slice(&u64::MAX.to_le_bytes());

        let path_ptr_pos = 100_u64.to_le_bytes();
        data[32..40].copy_from_slice(&path_ptr_pos);

        let cluster_ptr_pos = 120_u64.to_le_bytes();
        data[48..56].copy_from_slice(&cluster_ptr_pos);

        let mime_data = b"text/html\0image/png\0";
        data.extend_from_slice(mime_data);
        append_checksum(&mut data);

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
        write_header(&mut data);

        let cluster_count = 2_u32.to_le_bytes();
        data[28..32].copy_from_slice(&cluster_count);

        let path_ptr_pos = 90_u64.to_le_bytes();
        data[32..40].copy_from_slice(&path_ptr_pos);

        let cluster_ptr_pos = 100_u64.to_le_bytes();
        data[48..56].copy_from_slice(&cluster_ptr_pos);

        pad_to(&mut data, 90);
        pad_to(&mut data, 100);
        data.extend_from_slice(&[0u8; 16]);

        let c0_offset = data.len() as u64;
        let c1_offset = c0_offset + 20;

        data[100..108].copy_from_slice(&c0_offset.to_le_bytes());
        data[108..116].copy_from_slice(&c1_offset.to_le_bytes());

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
        let zstd_compressed =
            zstd::stream::encode_all(zstd_payload.as_slice(), 0).expect("Failed to compress test cluster");
        data.push(0x15);
        data.extend_from_slice(&zstd_compressed);

        append_checksum(&mut data);
        let zim = ZimFile::parse_bytes(Cursor::new(data)).expect("Parse failed");

        assert_eq!(zim.header.cluster_count, 2);
        assert_eq!(zim.cluster_pointers.len(), 2);
        assert_eq!(zim.cluster_pointers[0], c0_offset);
        assert_eq!(zim.cluster_pointers[1], c1_offset);
        assert_eq!(zim.cached_cluster_count(), 0);

        assert_eq!(zim.get_blob(0, 0), Some(vec![0xAA, 0xBB]));
        assert_eq!(zim.get_blob(1, 0), Some(vec![0xCC, 0xDD]));
        assert_eq!(zim.cluster_compression(0), Some(Compression::None));
        assert_eq!(zim.cluster_compression(1), Some(Compression::Zstd));
        assert_eq!(zim.cached_cluster_count(), 2);
    }

    #[test]
    fn test_parse_dirent_pointers_and_dirents() {
        let mut data = vec![0u8; HEADER_SIZE];
        write_header(&mut data);

        let article_count = 2_u32.to_le_bytes();
        data[24..28].copy_from_slice(&article_count);

        let path_ptr_pos = 90_u64.to_le_bytes();
        data[32..40].copy_from_slice(&path_ptr_pos);

        let cluster_ptr_pos = 120_u64.to_le_bytes();
        data[48..56].copy_from_slice(&cluster_ptr_pos);

        pad_to(&mut data, 90);
        let d0_ptr = 150_u64;
        let d1_ptr = 200_u64;
        data.extend_from_slice(&d0_ptr.to_le_bytes());
        data.extend_from_slice(&d1_ptr.to_le_bytes());

        pad_to(&mut data, 120);

        pad_to(&mut data, d0_ptr);
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

        let cluster_count = 0_u32.to_le_bytes();
        data[28..32].copy_from_slice(&cluster_count);

        append_checksum(&mut data);
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
        write_header(&mut data);

        let article_count = 1_u32.to_le_bytes();
        data[24..28].copy_from_slice(&article_count);

        let cluster_count = 1_u32.to_le_bytes();
        data[28..32].copy_from_slice(&cluster_count);

        let path_ptr_pos = 100_u64.to_le_bytes();
        data[32..40].copy_from_slice(&path_ptr_pos);

        let cluster_ptr_pos = 108_u64.to_le_bytes();
        data[48..56].copy_from_slice(&cluster_ptr_pos);

        pad_to(&mut data, 100);
        let d0_ptr = 130_u64;
        data.extend_from_slice(&d0_ptr.to_le_bytes());

        pad_to(&mut data, 108);
        data.extend_from_slice(&[0u8; 8]);

        let c0_offset = data.len() as u64;
        data[108..116].copy_from_slice(&c0_offset.to_le_bytes());

        data.push(0x01);
        data.extend_from_slice(&8u32.to_le_bytes());
        data.extend_from_slice(&13u32.to_le_bytes());
        data.extend(b"hello");

        pad_to(&mut data, d0_ptr);

        data.extend_from_slice(&1u16.to_le_bytes());
        data.push(0);
        data.push(b'C');
        data.extend_from_slice(&0u32.to_le_bytes());
        data.extend_from_slice(&0u32.to_le_bytes());
        data.extend_from_slice(&0u32.to_le_bytes());
        data.extend_from_slice(b"article\0Article\0");

        append_checksum(&mut data);
        let zim = ZimFile::parse_bytes(Cursor::new(data)).expect("Parse failed");

        let content = zim.get_content(&zim.dirents[0]).expect("content");
        assert_eq!(content, b"hello");
        assert_eq!(zim.get_mime_type(zim.dirents[0].mime_type), None);
    }

    #[test]
    fn test_metadata() {
        use crate::dirent::REDIRECT_MIME_TYPE;

        let mut data = vec![0u8; HEADER_SIZE];
        write_header(&mut data);

        let article_count = 3_u32.to_le_bytes();
        data[24..28].copy_from_slice(&article_count);

        let cluster_count = 1_u32.to_le_bytes();
        data[28..32].copy_from_slice(&cluster_count);

        let path_ptr_pos = 100_u64.to_le_bytes();
        data[32..40].copy_from_slice(&path_ptr_pos);

        let cluster_ptr_pos = 124_u64.to_le_bytes();
        data[48..56].copy_from_slice(&cluster_ptr_pos);

        pad_to(&mut data, 100);
        let d0_ptr = 157_u64;
        let d1_ptr = 184_u64;
        let d2_ptr = 213_u64;

        data.extend_from_slice(&d0_ptr.to_le_bytes());
        data.extend_from_slice(&d1_ptr.to_le_bytes());
        data.extend_from_slice(&d2_ptr.to_le_bytes());

        pad_to(&mut data, 124);
        data.extend_from_slice(&[0u8; 8]);

        let c0_offset = data.len() as u64;
        data[124..132].copy_from_slice(&c0_offset.to_le_bytes());

        data.push(0x01);
        data.extend_from_slice(&12u32.to_le_bytes());
        data.extend_from_slice(&17u32.to_le_bytes());
        data.extend_from_slice(&24u32.to_le_bytes());
        data.extend_from_slice(b"Kiwix");
        data.extend_from_slice(b"Offline");

        pad_to(&mut data, d0_ptr);
        data.extend_from_slice(&1u16.to_le_bytes());
        data.push(0);
        data.push(b'M');
        data.extend_from_slice(&0u32.to_le_bytes());
        data.extend_from_slice(&0u32.to_le_bytes());
        data.extend_from_slice(&0u32.to_le_bytes());
        data.extend_from_slice(b"Publisher\0\0");

        pad_to(&mut data, d1_ptr);
        data.extend_from_slice(&1u16.to_le_bytes());
        data.push(0);
        data.push(b'M');
        data.extend_from_slice(&0u32.to_le_bytes());
        data.extend_from_slice(&0u32.to_le_bytes());
        data.extend_from_slice(&1u32.to_le_bytes());
        data.extend_from_slice(b"Description\0\0");

        pad_to(&mut data, d2_ptr);
        data.extend_from_slice(&REDIRECT_MIME_TYPE.to_le_bytes());
        data.push(0);
        data.push(b'M');
        data.extend_from_slice(&0u32.to_le_bytes());
        data.extend_from_slice(&0u32.to_le_bytes());
        data.extend_from_slice(b"Title\0\0");

        append_checksum(&mut data);
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
        write_header(&mut data);

        let cluster_count = 2_u32.to_le_bytes();
        data[28..32].copy_from_slice(&cluster_count);

        let path_ptr_pos = 90_u64.to_le_bytes();
        data[32..40].copy_from_slice(&path_ptr_pos);

        let cluster_ptr_pos = 100_u64.to_le_bytes();
        data[48..56].copy_from_slice(&cluster_ptr_pos);

        pad_to(&mut data, 90);
        pad_to(&mut data, 100);
        data.extend_from_slice(&[0u8; 16]);

        let c0_offset = data.len() as u64;
        let c1_offset = c0_offset + 24;

        data[100..108].copy_from_slice(&c0_offset.to_le_bytes());
        data[108..116].copy_from_slice(&c1_offset.to_le_bytes());

        data.push(0x01);
        data.extend_from_slice(&8u32.to_le_bytes());
        data.extend_from_slice(&13u32.to_le_bytes());
        data.extend(b"first");

        pad_to(&mut data, c1_offset);

        data.push(0x01);
        data.extend_from_slice(&8u32.to_le_bytes());
        data.extend_from_slice(&14u32.to_le_bytes());
        data.extend(b"second");

        append_checksum(&mut data);
        let zim = ZimFile::parse_bytes_with_cache_capacity(Cursor::new(data), 1).expect("Parse failed");

        assert_eq!(zim.get_blob(0, 0), Some(b"first".to_vec()));
        assert_eq!(zim.cached_cluster_count(), 1);

        assert_eq!(zim.get_blob(1, 0), Some(b"second".to_vec()));
        assert_eq!(zim.cached_cluster_count(), 1);

        assert_eq!(zim.get_blob(0, 0), Some(b"first".to_vec()));
        assert_eq!(zim.get_blob(1, 0), Some(b"second".to_vec()));
    }

    #[test]
    fn test_empty_zim_checksum_valid() {
        let data = empty_zim_archive_content();
        let zim = ZimFile::parse_bytes(Cursor::new(data)).expect("Parse failed");

        assert!(zim.has_checksum());
        assert!(zim.check());
        assert!(zim.check_integrity(IntegrityCheck::Checksum));
        assert_eq!(
            zim.get_checksum(),
            Some("9f3ecd9546f6c53b35b4c6d4c08ed066".to_string())
        );
    }

    #[test]
    fn test_empty_zim_wrong_checksum() {
        let mut data = empty_zim_archive_content();
        let last = data.len() - 1;
        data[last] ^= 0xff;

        let zim = ZimFile::parse_bytes(Cursor::new(data)).expect("Parse failed");
        assert!(!zim.check());
        assert!(!zim.check_integrity(IntegrityCheck::Checksum));
    }

    #[test]
    fn test_bad_checksum_size_rejected_on_open() {
        let mut data = empty_zim_archive_content();
        data.pop();

        let result = ZimFile::parse_bytes(Cursor::new(data));
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "Zim file is of bad size or corrupted.");
    }

    #[test]
    fn test_no_checksum_returns_false() {
        let mut data = vec![0u8; HEADER_SIZE];
        data[0..4].copy_from_slice(b"ZIM\x04");
        let mime_list_pos = 72_u64.to_le_bytes();
        data[56..64].copy_from_slice(&mime_list_pos);
        data[40..48].copy_from_slice(&u64::MAX.to_le_bytes());
        let path_ptr_pos = 80_u64.to_le_bytes();
        data[32..40].copy_from_slice(&path_ptr_pos);
        let cluster_ptr_pos = 80_u64.to_le_bytes();
        data[48..56].copy_from_slice(&cluster_ptr_pos);

        let zim = ZimFile::parse_bytes(Cursor::new(data)).expect("Parse failed");
        assert!(!zim.has_checksum());
        assert!(!zim.check());
        assert_eq!(zim.get_checksum(), None);
    }

    #[test]
    fn test_check_dirent_ptrs() {
        let mut data = vec![0u8; HEADER_SIZE];
        write_header(&mut data);

        let article_count = 1_u32.to_le_bytes();
        data[24..28].copy_from_slice(&article_count);

        let path_ptr_pos = 90_u64.to_le_bytes();
        data[32..40].copy_from_slice(&path_ptr_pos);
        let cluster_ptr_pos = 98_u64.to_le_bytes();
        data[48..56].copy_from_slice(&cluster_ptr_pos);

        pad_to(&mut data, 90);
        let d0_ptr = 150_u64;
        data.extend_from_slice(&d0_ptr.to_le_bytes());

        pad_to(&mut data, 98);

        pad_to(&mut data, d0_ptr);
        data.extend_from_slice(&1u16.to_le_bytes());
        data.push(0);
        data.push(b'A');
        data.extend_from_slice(&0u32.to_le_bytes());
        data.extend_from_slice(&0u32.to_le_bytes());
        data.extend_from_slice(&0u32.to_le_bytes());
        data.extend_from_slice(b"a\0a\0");

        append_checksum(&mut data);
        let zim = ZimFile::parse_bytes(Cursor::new(data)).expect("Parse failed");
        assert!(zim.check_integrity(IntegrityCheck::DirentPtrs));
    }

    #[test]
    fn test_check_dirent_ptrs_invalid() {
        let mut data = vec![0u8; HEADER_SIZE];
        write_header(&mut data);

        let article_count = 1_u32.to_le_bytes();
        data[24..28].copy_from_slice(&article_count);
        let path_ptr_pos = 90_u64.to_le_bytes();
        data[32..40].copy_from_slice(&path_ptr_pos);
        let cluster_ptr_pos = 98_u64.to_le_bytes();
        data[48..56].copy_from_slice(&cluster_ptr_pos);

        pad_to(&mut data, 90);
        data.extend_from_slice(&10_u64.to_le_bytes());

        pad_to(&mut data, 98);

        append_checksum(&mut data);
        let zim = ZimFile::parse_bytes(Cursor::new(data)).expect("Parse failed");
        assert!(!zim.check_integrity(IntegrityCheck::DirentPtrs));
    }

    #[test]
    fn test_check_dirent_order() {
        let mut data = vec![0u8; HEADER_SIZE];
        write_header(&mut data);

        let article_count = 2_u32.to_le_bytes();
        data[24..28].copy_from_slice(&article_count);
        let path_ptr_pos = 90_u64.to_le_bytes();
        data[32..40].copy_from_slice(&path_ptr_pos);
        let cluster_ptr_pos = 106_u64.to_le_bytes();
        data[48..56].copy_from_slice(&cluster_ptr_pos);

        pad_to(&mut data, 90);
        let d0_ptr = 150_u64;
        let d1_ptr = 180_u64;
        data.extend_from_slice(&d0_ptr.to_le_bytes());
        data.extend_from_slice(&d1_ptr.to_le_bytes());

        pad_to(&mut data, 106);

        pad_to(&mut data, d0_ptr);
        data.extend_from_slice(&1u16.to_le_bytes());
        data.push(0);
        data.push(b'A');
        data.extend_from_slice(&0u32.to_le_bytes());
        data.extend_from_slice(&0u32.to_le_bytes());
        data.extend_from_slice(&0u32.to_le_bytes());
        data.extend_from_slice(b"a\0a\0");

        while data.len() < d1_ptr as usize {
            data.push(0);
        }
        data.extend_from_slice(&1u16.to_le_bytes());
        data.push(0);
        data.push(b'A');
        data.extend_from_slice(&0u32.to_le_bytes());
        data.extend_from_slice(&0u32.to_le_bytes());
        data.extend_from_slice(&0u32.to_le_bytes());
        data.extend_from_slice(b"b\0b\0");

        append_checksum(&mut data);
        let zim = ZimFile::parse_bytes(Cursor::new(data)).expect("Parse failed");
        assert!(zim.check_integrity(IntegrityCheck::DirentOrder));
    }

    #[test]
    fn test_check_dirent_order_invalid() {
        let mut data = vec![0u8; HEADER_SIZE];
        write_header(&mut data);

        let article_count = 2_u32.to_le_bytes();
        data[24..28].copy_from_slice(&article_count);
        let path_ptr_pos = 90_u64.to_le_bytes();
        data[32..40].copy_from_slice(&path_ptr_pos);
        let cluster_ptr_pos = 106_u64.to_le_bytes();
        data[48..56].copy_from_slice(&cluster_ptr_pos);

        pad_to(&mut data, 90);
        let d0_ptr = 150_u64;
        let d1_ptr = 180_u64;
        data.extend_from_slice(&d0_ptr.to_le_bytes());
        data.extend_from_slice(&d1_ptr.to_le_bytes());

        pad_to(&mut data, 106);

        pad_to(&mut data, d0_ptr);
        data.extend_from_slice(&1u16.to_le_bytes());
        data.push(0);
        data.push(b'A');
        data.extend_from_slice(&0u32.to_le_bytes());
        data.extend_from_slice(&0u32.to_le_bytes());
        data.extend_from_slice(&0u32.to_le_bytes());
        data.extend_from_slice(b"b\0b\0");

        pad_to(&mut data, d1_ptr);
        data.extend_from_slice(&1u16.to_le_bytes());
        data.push(0);
        data.push(b'A');
        data.extend_from_slice(&0u32.to_le_bytes());
        data.extend_from_slice(&0u32.to_le_bytes());
        data.extend_from_slice(&0u32.to_le_bytes());
        data.extend_from_slice(b"a\0a\0");

        append_checksum(&mut data);
        let zim = ZimFile::parse_bytes(Cursor::new(data)).expect("Parse failed");
        assert!(!zim.check_integrity(IntegrityCheck::DirentOrder));
    }

    #[test]
    fn test_check_cluster_ptrs_and_offsets() {
        let mut data = vec![0u8; HEADER_SIZE];
        write_header(&mut data);

        let cluster_count = 1_u32.to_le_bytes();
        data[28..32].copy_from_slice(&cluster_count);
        let path_ptr_pos = 90_u64.to_le_bytes();
        data[32..40].copy_from_slice(&path_ptr_pos);
        let cluster_ptr_pos = 98_u64.to_le_bytes();
        data[48..56].copy_from_slice(&cluster_ptr_pos);

        pad_to(&mut data, 98);
        data.extend_from_slice(&[0u8; 8]);

        let c0_offset = data.len() as u64;
        data[98..106].copy_from_slice(&c0_offset.to_le_bytes());

        data.push(0x01);
        data.extend_from_slice(&8u32.to_le_bytes());
        data.extend_from_slice(&13u32.to_le_bytes());
        data.extend(b"payload");

        append_checksum(&mut data);
        let zim = ZimFile::parse_bytes(Cursor::new(data)).expect("Parse failed");
        assert!(zim.check_integrity(IntegrityCheck::ClusterPtrs));
        assert!(zim.check_integrity(IntegrityCheck::ClustersOffsets));
    }

    #[test]
    fn test_check_cluster_ptrs_invalid() {
        let mut data = vec![0u8; HEADER_SIZE];
        write_header(&mut data);

        let cluster_count = 1_u32.to_le_bytes();
        data[28..32].copy_from_slice(&cluster_count);
        let path_ptr_pos = 90_u64.to_le_bytes();
        data[32..40].copy_from_slice(&path_ptr_pos);
        let cluster_ptr_pos = 98_u64.to_le_bytes();
        data[48..56].copy_from_slice(&cluster_ptr_pos);

        pad_to(&mut data, 90);
        data.extend_from_slice(&10_u64.to_le_bytes());

        pad_to(&mut data, 98);

        append_checksum(&mut data);
        let zim = ZimFile::parse_bytes(Cursor::new(data)).expect("Parse failed");
        assert!(!zim.check_integrity(IntegrityCheck::ClusterPtrs));
        assert!(!zim.check_integrity(IntegrityCheck::ClustersOffsets));
    }

    #[test]
    fn test_check_dirent_mime_types() {
        let mut data = vec![0u8; HEADER_SIZE];
        write_header(&mut data);

        let article_count = 1_u32.to_le_bytes();
        data[24..28].copy_from_slice(&article_count);
        let path_ptr_pos = 90_u64.to_le_bytes();
        data[32..40].copy_from_slice(&path_ptr_pos);
        let cluster_ptr_pos = 98_u64.to_le_bytes();
        data[48..56].copy_from_slice(&cluster_ptr_pos);

        data.truncate(80);
        data.extend_from_slice(b"text/html\0");
        pad_to(&mut data, 90);
        let d0_ptr = 150_u64;
        data.extend_from_slice(&d0_ptr.to_le_bytes());
        pad_to(&mut data, 98);
        pad_to(&mut data, d0_ptr);
        data.extend_from_slice(&0u16.to_le_bytes());
        data.push(0);
        data.push(b'C');
        data.extend_from_slice(&0u32.to_le_bytes());
        data.extend_from_slice(&0u32.to_le_bytes());
        data.extend_from_slice(&0u32.to_le_bytes());
        data.extend_from_slice(b"article\0Article\0");

        append_checksum(&mut data);
        let zim = ZimFile::parse_bytes(Cursor::new(data)).expect("Parse failed");
        assert!(zim.check_integrity(IntegrityCheck::DirentMimeTypes));
    }

    #[test]
    fn test_check_dirent_mime_types_invalid() {
        let mut data = vec![0u8; HEADER_SIZE];
        write_header(&mut data);

        let article_count = 1_u32.to_le_bytes();
        data[24..28].copy_from_slice(&article_count);
        let path_ptr_pos = 90_u64.to_le_bytes();
        data[32..40].copy_from_slice(&path_ptr_pos);
        let cluster_ptr_pos = 98_u64.to_le_bytes();
        data[48..56].copy_from_slice(&cluster_ptr_pos);

        data.truncate(80);
        data.extend_from_slice(b"text/html\0");
        pad_to(&mut data, 90);
        let d0_ptr = 150_u64;
        data.extend_from_slice(&d0_ptr.to_le_bytes());
        pad_to(&mut data, 98);
        pad_to(&mut data, d0_ptr);
        data.extend_from_slice(&99u16.to_le_bytes());
        data.push(0);
        data.push(b'C');
        data.extend_from_slice(&0u32.to_le_bytes());
        data.extend_from_slice(&0u32.to_le_bytes());
        data.extend_from_slice(&0u32.to_le_bytes());
        data.extend_from_slice(b"article\0Article\0");

        append_checksum(&mut data);
        let zim = ZimFile::parse_bytes(Cursor::new(data)).expect("Parse failed");
        assert!(!zim.check_integrity(IntegrityCheck::DirentMimeTypes));
    }

    #[test]
    fn test_check_title_index_v0_and_v1() {
        let mut data = vec![0u8; HEADER_SIZE];
        write_header(&mut data);

        let article_count = 3_u32.to_le_bytes();
        data[24..28].copy_from_slice(&article_count);
        let cluster_count = 1_u32.to_le_bytes();
        data[28..32].copy_from_slice(&cluster_count);

        let title_idx_pos = 90_u64;
        data[40..48].copy_from_slice(&title_idx_pos.to_le_bytes());
        let path_ptr_pos = 102_u64;
        data[32..40].copy_from_slice(&path_ptr_pos.to_le_bytes());
        let cluster_ptr_pos = 126_u64;
        data[48..56].copy_from_slice(&cluster_ptr_pos.to_le_bytes());

        while data.len() < 80 {
            data.push(0);
        }
        data.push(0);

        while data.len() < title_idx_pos as usize {
            data.push(0);
        }
        data.extend_from_slice(&0u32.to_le_bytes());
        data.extend_from_slice(&1u32.to_le_bytes());
        data.extend_from_slice(&2u32.to_le_bytes());

        let d0_ptr = 150_u64;
        let d1_ptr = 180_u64;
        let listing_ptr = 210_u64;
        data.extend_from_slice(&d0_ptr.to_le_bytes());
        data.extend_from_slice(&d1_ptr.to_le_bytes());
        data.extend_from_slice(&listing_ptr.to_le_bytes());

        let c0_offset = 250_u64;
        data.extend_from_slice(&c0_offset.to_le_bytes());

        while data.len() < d0_ptr as usize {
            data.push(0);
        }
        data.extend_from_slice(&1u16.to_le_bytes());
        data.push(0);
        data.push(b'A');
        data.extend_from_slice(&0u32.to_le_bytes());
        data.extend_from_slice(&0u32.to_le_bytes());
        data.extend_from_slice(&0u32.to_le_bytes());
        data.extend_from_slice(b"a\0Alpha\0");

        while data.len() < d1_ptr as usize {
            data.push(0);
        }
        data.extend_from_slice(&1u16.to_le_bytes());
        data.push(0);
        data.push(b'A');
        data.extend_from_slice(&0u32.to_le_bytes());
        data.extend_from_slice(&0u32.to_le_bytes());
        data.extend_from_slice(&0u32.to_le_bytes());
        data.extend_from_slice(b"b\0Beta\0");

        while data.len() < listing_ptr as usize {
            data.push(0);
        }
        data.extend_from_slice(&1u16.to_le_bytes());
        data.push(0);
        data.push(b'X');
        data.extend_from_slice(&0u32.to_le_bytes());
        data.extend_from_slice(&0u32.to_le_bytes());
        data.extend_from_slice(&1u32.to_le_bytes());
        data.extend_from_slice(b"listing/titleOrdered/v1\0\0");

        while data.len() < c0_offset as usize {
            data.push(0);
        }
        data.push(0x01);
        data.extend_from_slice(&12u32.to_le_bytes());
        data.extend_from_slice(&16u32.to_le_bytes());
        data.extend_from_slice(&28u32.to_le_bytes());
        data.extend_from_slice(&0u32.to_le_bytes());
        data.extend_from_slice(&0u32.to_le_bytes());
        data.extend_from_slice(&1u32.to_le_bytes());
        data.extend_from_slice(&2u32.to_le_bytes());

        append_checksum(&mut data);
        let zim = ZimFile::parse_bytes(Cursor::new(data)).expect("Parse failed");
        assert!(zim.check_integrity(IntegrityCheck::TitleIndex));
    }

    #[test]
    fn test_check_title_index_invalid() {
        let mut data = vec![0u8; HEADER_SIZE];
        write_header(&mut data);

        let article_count = 2_u32.to_le_bytes();
        data[24..28].copy_from_slice(&article_count);

        let title_idx_pos = 90_u64;
        data[40..48].copy_from_slice(&title_idx_pos.to_le_bytes());
        let path_ptr_pos = 98_u64.to_le_bytes();
        data[32..40].copy_from_slice(&path_ptr_pos);
        let cluster_ptr_pos = 106_u64.to_le_bytes();
        data[48..56].copy_from_slice(&cluster_ptr_pos);

        while data.len() < title_idx_pos as usize {
            data.push(0);
        }
        data.extend_from_slice(&1u32.to_le_bytes());
        data.extend_from_slice(&0u32.to_le_bytes());

        let d0_ptr = 150_u64;
        let d1_ptr = 180_u64;
        data.extend_from_slice(&d0_ptr.to_le_bytes());
        data.extend_from_slice(&d1_ptr.to_le_bytes());

        while data.len() < d0_ptr as usize {
            data.push(0);
        }
        data.extend_from_slice(&1u16.to_le_bytes());
        data.push(0);
        data.push(b'A');
        data.extend_from_slice(&0u32.to_le_bytes());
        data.extend_from_slice(&0u32.to_le_bytes());
        data.extend_from_slice(&0u32.to_le_bytes());
        data.extend_from_slice(b"a\0Alpha\0");

        while data.len() < d1_ptr as usize {
            data.push(0);
        }
        data.extend_from_slice(&1u16.to_le_bytes());
        data.push(0);
        data.push(b'A');
        data.extend_from_slice(&0u32.to_le_bytes());
        data.extend_from_slice(&0u32.to_le_bytes());
        data.extend_from_slice(&0u32.to_le_bytes());
        data.extend_from_slice(b"b\0Beta\0");

        append_checksum(&mut data);
        let zim = ZimFile::parse_bytes(Cursor::new(data)).expect("Parse failed");
        assert!(!zim.check_integrity(IntegrityCheck::TitleIndex));
    }

    #[test]
    fn test_dirent_long_path_and_pseudo_title() {
        let mut dirent = Dirent {
            mime_type: 1,
            extra_len: 0,
            namespace: 'C',
            revision: 0,
            data: DirentData::Content {
                cluster_number: 0,
                blob_number: 0,
            },
            url: "foo".to_string(),
            title: "Bar".to_string(),
            parameter: Vec::new(),
        };

        assert_eq!(dirent.long_path(), "C/foo");
        assert_eq!(dirent.pseudo_title(), "C/Bar");
        dirent.title.clear();
        assert_eq!(dirent.pseudo_title(), "C/foo");
    }
}
