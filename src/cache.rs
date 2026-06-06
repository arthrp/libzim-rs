use std::collections::{HashMap, VecDeque};

use crate::cluster::Cluster;

pub struct ClusterCache {
    capacity: usize,
    map: HashMap<usize, Cluster>,
    order: VecDeque<usize>,
}

impl ClusterCache {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            map: HashMap::new(),
            order: VecDeque::new(),
        }
    }

    pub fn contains(&self, idx: usize) -> bool {
        self.map.contains_key(&idx)
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn get(&mut self, idx: usize) -> Option<&Cluster> {
        if self.map.contains_key(&idx) {
            self.touch(idx);
            self.map.get(&idx)
        } else {
            None
        }
    }

    pub fn put(&mut self, idx: usize, cluster: Cluster) {
        if self.map.insert(idx, cluster).is_some() {
            self.touch(idx);
        } else {
            self.order.push_back(idx);
        }

        while self.map.len() > self.capacity {
            if let Some(old) = self.order.pop_front() {
                self.map.remove(&old);
            }
        }
    }

    fn touch(&mut self, idx: usize) {
        if let Some(pos) = self.order.iter().position(|&k| k == idx) {
            self.order.remove(pos);
            self.order.push_back(idx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cluster::{Cluster, Compression};
    use std::io::Cursor;

    fn make_cluster(compression_byte: u8, payload: Vec<u8>) -> Cluster {
        let mut data = Vec::new();
        data.push(compression_byte);
        data.extend(payload);
        let mut reader = Cursor::new(data);
        Cluster::parse(&mut reader).expect("Failed to parse test cluster")
    }

    fn uncompressed_payload() -> Vec<u8> {
        let mut payload = Vec::new();
        payload.extend_from_slice(&8u32.to_le_bytes());
        payload.extend_from_slice(&10u32.to_le_bytes());
        payload.extend(vec![0xAA, 0xBB]);
        payload
    }

    #[test]
    fn test_capacity_minimum_one() {
        let mut cache = ClusterCache::new(0);
        cache.put(0, make_cluster(0x01, uncompressed_payload()));
        cache.put(1, make_cluster(0x01, uncompressed_payload()));
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn test_put_and_get() {
        let mut cache = ClusterCache::new(2);
        cache.put(0, make_cluster(0x01, uncompressed_payload()));
        cache.put(1, make_cluster(0x01, uncompressed_payload()));

        assert_eq!(cache.len(), 2);
        assert!(cache.get(0).is_some());
        assert!(cache.get(1).is_some());
    }

    #[test]
    fn test_eviction_removes_lru() {
        let mut cache = ClusterCache::new(2);
        cache.put(0, make_cluster(0x01, uncompressed_payload()));
        cache.put(1, make_cluster(0x01, uncompressed_payload()));
        cache.put(2, make_cluster(0x01, uncompressed_payload()));

        assert_eq!(cache.len(), 2);
        assert!(!cache.contains(0));
        assert!(cache.contains(1));
        assert!(cache.contains(2));
    }

    #[test]
    fn test_get_promotes_to_mru() {
        let mut cache = ClusterCache::new(2);
        cache.put(0, make_cluster(0x01, uncompressed_payload()));
        cache.put(1, make_cluster(0x01, uncompressed_payload()));

        assert!(cache.get(0).is_some());

        cache.put(2, make_cluster(0x01, uncompressed_payload()));

        assert!(!cache.contains(1));
        assert!(cache.contains(0));
        assert!(cache.contains(2));
    }

    #[test]
    fn test_put_update_existing() {
        let mut cache = ClusterCache::new(2);
        cache.put(0, make_cluster(0x01, uncompressed_payload()));
        cache.put(0, make_cluster(0x01, uncompressed_payload()));

        assert_eq!(cache.len(), 1);
        assert!(cache.contains(0));
    }

    #[test]
    fn test_cluster_blob_accessible_from_cache() {
        let mut cache = ClusterCache::new(1);
        cache.put(0, make_cluster(0x01, uncompressed_payload()));

        let cluster = cache.get(0).expect("cluster");
        assert_eq!(cluster.compression, Compression::None);
        assert_eq!(cluster.get_blob(0), Some(&[0xAA, 0xBB][..]));
    }
}
