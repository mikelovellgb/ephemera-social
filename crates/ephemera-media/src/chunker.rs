//! Content chunking for distributed storage.
//!
//! Splits media content (images and video) into fixed-size chunks for
//! distribution across the Ephemera P2P network. Each chunk is
//! independently hashed with BLAKE3 for integrity verification.

use ephemera_types::ContentId;

use crate::MediaError;

/// Default chunk size: 256 KiB (262,144 bytes).
pub const DEFAULT_CHUNK_SIZE: u32 = 256 * 1024;

/// A single chunk of content data.
#[derive(Debug, Clone)]
pub struct Chunk {
    /// Zero-based index of this chunk within the content.
    pub index: u32,
    /// BLAKE3 hash of this chunk's data.
    pub hash: ContentId,
    /// The raw chunk bytes.
    pub data: Vec<u8>,
    /// Actual byte count (may be less than chunk size for the last chunk).
    pub size: usize,
}

/// Summary information about a chunk (without the data payload).
#[derive(Debug, Clone)]
pub struct ChunkInfo {
    /// Zero-based index.
    pub index: u32,
    /// BLAKE3 hash of this chunk's data.
    pub hash: ContentId,
    /// Actual byte count.
    pub size: u32,
}

/// Manifest describing how content was chunked.
///
/// Used by receivers to reassemble and verify content integrity.
#[derive(Debug, Clone)]
pub struct ChunkManifest {
    /// BLAKE3 hash of the complete, un-chunked content.
    pub content_hash: ContentId,
    /// Total size of the original content in bytes.
    pub total_size: u64,
    /// Number of chunks.
    pub chunk_count: u32,
    /// Standard chunk size in bytes (last chunk may be smaller).
    pub chunk_size: u32,
    /// Ordered list of chunk metadata.
    pub chunks: Vec<ChunkInfo>,
}

/// Content chunker for splitting and reassembling media data.
pub struct ContentChunker;

impl ContentChunker {
    /// Split content into fixed-size chunks for distributed storage.
    ///
    /// Each chunk is 256 KiB (262,144 bytes) except the last, which
    /// contains the remainder. Each chunk gets a BLAKE3 hash for
    /// integrity verification during reassembly.
    ///
    /// Returns an empty vec if the input is empty.
    #[must_use]
    pub fn chunk(data: &[u8]) -> Vec<Chunk> {
        Self::chunk_with_size(data, DEFAULT_CHUNK_SIZE)
    }

    /// Split content using a custom chunk size.
    ///
    /// Useful for testing with smaller chunks.
    #[must_use]
    pub fn chunk_with_size(data: &[u8], chunk_size: u32) -> Vec<Chunk> {
        let cs = chunk_size as usize;
        data.chunks(cs)
            .enumerate()
            .map(|(i, slice)| {
                let hash_bytes = blake3::hash(slice);
                let hash = ContentId::from_digest(*hash_bytes.as_bytes());
                Chunk {
                    index: i as u32,
                    hash,
                    data: slice.to_vec(),
                    size: slice.len(),
                }
            })
            .collect()
    }

    /// Build a `ChunkManifest` from content data.
    ///
    /// Computes the overall content hash and per-chunk metadata.
    #[must_use]
    pub fn manifest(data: &[u8]) -> ChunkManifest {
        Self::manifest_with_size(data, DEFAULT_CHUNK_SIZE)
    }

    /// Build a manifest with a custom chunk size.
    #[must_use]
    pub fn manifest_with_size(data: &[u8], chunk_size: u32) -> ChunkManifest {
        let content_hash_bytes = blake3::hash(data);
        let content_hash = ContentId::from_digest(*content_hash_bytes.as_bytes());

        let chunks = Self::chunk_with_size(data, chunk_size);
        let chunk_infos: Vec<ChunkInfo> = chunks
            .iter()
            .map(|c| ChunkInfo {
                index: c.index,
                hash: c.hash.clone(),
                size: c.size as u32,
            })
            .collect();

        ChunkManifest {
            content_hash,
            total_size: data.len() as u64,
            chunk_count: chunk_infos.len() as u32,
            chunk_size,
            chunks: chunk_infos,
        }
    }

    /// Reassemble chunks back into the original content.
    ///
    /// Verifies the BLAKE3 hash of each chunk before assembly.
    /// Chunks must be provided in order (sorted by index).
    ///
    /// # Errors
    ///
    /// Returns [`MediaError`] if:
    /// - Any chunk's hash does not match its data
    /// - Chunks are not contiguous (gap or duplicate indices)
    pub fn reassemble(chunks: &[Chunk]) -> Result<Vec<u8>, MediaError> {
        if chunks.is_empty() {
            return Ok(Vec::new());
        }

        // Verify ordering and contiguity.
        for (i, chunk) in chunks.iter().enumerate() {
            if chunk.index != i as u32 {
                return Err(MediaError::Validation(format!(
                    "chunk index mismatch: expected {i}, got {}",
                    chunk.index
                )));
            }
        }

        // Verify hashes and collect data.
        let mut assembled = Vec::new();
        for chunk in chunks {
            let computed = blake3::hash(&chunk.data);
            let expected = ContentId::from_digest(*computed.as_bytes());
            if expected != chunk.hash {
                return Err(MediaError::Validation(format!(
                    "chunk {} hash mismatch: data has been tampered with",
                    chunk.index
                )));
            }
            assembled.extend_from_slice(&chunk.data);
        }

        Ok(assembled)
    }

    /// Reassemble and verify against a manifest.
    ///
    /// After reassembly, verifies the overall content hash matches
    /// the manifest's `content_hash`.
    ///
    /// # Errors
    ///
    /// Returns [`MediaError`] if chunk or content hash verification fails.
    pub fn reassemble_with_manifest(
        chunks: &[Chunk],
        manifest: &ChunkManifest,
    ) -> Result<Vec<u8>, MediaError> {
        if chunks.len() as u32 != manifest.chunk_count {
            return Err(MediaError::Validation(format!(
                "expected {} chunks, got {}",
                manifest.chunk_count,
                chunks.len()
            )));
        }

        let data = Self::reassemble(chunks)?;

        // Verify overall content hash.
        let computed = blake3::hash(&data);
        let computed_hash = ContentId::from_digest(*computed.as_bytes());
        if computed_hash != manifest.content_hash {
            return Err(MediaError::Validation(
                "reassembled content hash does not match manifest".into(),
            ));
        }

        Ok(data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunk_empty_data() {
        let chunks = ContentChunker::chunk(&[]);
        assert!(chunks.is_empty());
    }

    #[test]
    fn chunk_small_data_single_chunk() {
        let data = vec![42u8; 100];
        let chunks = ContentChunker::chunk(&data);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].index, 0);
        assert_eq!(chunks[0].size, 100);
        assert_eq!(chunks[0].data, data);
    }

    #[test]
    fn chunk_and_reassemble_roundtrip() {
        let data: Vec<u8> = (0u8..=255).cycle().take(1000).collect();
        let chunks = ContentChunker::chunk_with_size(&data, 256);
        assert_eq!(chunks.len(), 4); // 1000 / 256 = 3 full + 1 partial
        let reassembled = ContentChunker::reassemble(&chunks).unwrap();
        assert_eq!(reassembled, data);
    }

    #[test]
    fn chunk_exact_boundary() {
        let data = vec![0xABu8; 512];
        let chunks = ContentChunker::chunk_with_size(&data, 256);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].size, 256);
        assert_eq!(chunks[1].size, 256);
        let reassembled = ContentChunker::reassemble(&chunks).unwrap();
        assert_eq!(reassembled, data);
    }

    #[test]
    fn tampered_chunk_fails_reassembly() {
        let data = vec![1u8; 600];
        let mut chunks = ContentChunker::chunk_with_size(&data, 256);
        // Tamper with the second chunk's data.
        chunks[1].data[0] = 0xFF;
        let result = ContentChunker::reassemble(&chunks);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("tampered"));
    }

    #[test]
    fn manifest_roundtrip() {
        let data: Vec<u8> = (0..200).map(|i| (i % 256) as u8).collect();
        let manifest = ContentChunker::manifest_with_size(&data, 64);
        assert_eq!(manifest.total_size, 200);
        assert_eq!(manifest.chunk_count, 4); // 200/64 = 3 full + 1 partial
        assert_eq!(manifest.chunk_size, 64);

        let chunks = ContentChunker::chunk_with_size(&data, 64);
        let reassembled = ContentChunker::reassemble_with_manifest(&chunks, &manifest).unwrap();
        assert_eq!(reassembled, data);
    }

    #[test]
    fn manifest_rejects_wrong_content() {
        let data = vec![1u8; 100];
        let manifest = ContentChunker::manifest_with_size(&data, 64);

        // Different data, same chunk structure.
        let wrong_data = vec![2u8; 100];
        let wrong_chunks = ContentChunker::chunk_with_size(&wrong_data, 64);
        let result = ContentChunker::reassemble_with_manifest(&wrong_chunks, &manifest);
        assert!(result.is_err());
    }

    #[test]
    fn manifest_rejects_wrong_chunk_count() {
        let data = vec![1u8; 100];
        let manifest = ContentChunker::manifest_with_size(&data, 64);
        let chunks = ContentChunker::chunk_with_size(&data, 32); // More chunks
        let result = ContentChunker::reassemble_with_manifest(&chunks, &manifest);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("expected"));
    }
}
