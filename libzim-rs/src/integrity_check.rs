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
