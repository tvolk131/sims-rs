//! Chunked, compressed REDB snapshot transfer framing for mid-game join and
//! drift recovery.
//!
//! Snapshots carry both the deterministic [`SessionState`] at `at_tick` and the
//! host's REDB world snapshot bytes inside [`SnapshotEnvelope`], compressed
//! with deflate and split into fixed-size chunks with end-to-end SHA-256
//! integrity checks and bounded decompression to prevent zip-bomb OOMs.

use crate::error::{ProtocolError, SnapshotError};
use crate::manifest::sha256_bytes;
use crate::protocol::{Epoch, SnapshotId, StateFingerprint, Tick};
use crate::session::SessionState;
use flate2::Compression;
use flate2::read::DeflateDecoder;
use flate2::write::DeflateEncoder;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::{Read, Write};

/// Maximum uncompressed snapshot payload size (64 MiB).
pub const MAX_SNAPSHOT_UNCOMPRESSED_BYTES: u64 = 64 * 1024 * 1024;

/// Maximum compressed snapshot payload size (64 MiB + 1 MiB framing overhead).
pub const MAX_SNAPSHOT_COMPRESSED_BYTES: u64 = MAX_SNAPSHOT_UNCOMPRESSED_BYTES + 1024 * 1024;

/// Maximum single snapshot chunk size (1 MiB, well below `MAX_WIRE_FRAME_BYTES`).
pub const MAX_SNAPSHOT_CHUNK_BYTES: u32 = 1024 * 1024;

/// Maximum number of chunks in a single snapshot transfer, bounding chunk-map
/// metadata overhead against tiny-chunk memory amplification attacks.
pub const MAX_SNAPSHOT_CHUNKS: u32 = 65_536;

/// Compression algorithm applied to a snapshot payload.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SnapshotCompression {
    /// Raw uncompressed bytes.
    None,
    /// RFC 1951 deflate compression.
    #[default]
    Deflate,
}

/// Payload bundled inside a snapshot transfer so the receiver atomically
/// restores both deterministic session state and REDB world bytes at `at_tick`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotEnvelope {
    /// Deterministic co-op session state captured at `at_tick`.
    pub session: SessionState,
    /// Raw REDB snapshot bytes captured by the host at `at_tick`.
    pub redb_bytes: Vec<u8>,
}

impl SnapshotEnvelope {
    /// Encodes the envelope with `postcard`.
    pub fn encode(&self) -> Result<Vec<u8>, SnapshotError> {
        postcard::to_allocvec(self)
            .map_err(|e| SnapshotError::Envelope(ProtocolError::Encode(e.to_string())))
    }

    /// Decodes the envelope from `postcard` bytes, rejecting trailing bytes.
    pub fn decode(bytes: &[u8]) -> Result<Self, SnapshotError> {
        let (env, rest) = postcard::take_from_bytes(bytes)
            .map_err(|e| SnapshotError::Envelope(ProtocolError::Decode(e.to_string())))?;
        if !rest.is_empty() {
            return Err(SnapshotError::Envelope(ProtocolError::Decode(format!(
                "unexpected {} trailing bytes in snapshot envelope",
                rest.len()
            ))));
        }
        Ok(env)
    }
}

/// Metadata announcing a chunked snapshot transfer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotHeader {
    pub snapshot_id: SnapshotId,
    pub epoch: Epoch,
    pub at_tick: Tick,
    pub state_fingerprint: StateFingerprint,
    pub compression: SnapshotCompression,
    pub uncompressed_len: u64,
    pub compressed_len: u64,
    pub uncompressed_sha256: [u8; 32],
    pub compressed_sha256: [u8; 32],
    pub chunk_size: u32,
    pub total_chunks: u32,
}

/// One chunk of a snapshot transfer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotChunk {
    pub snapshot_id: SnapshotId,
    pub chunk_index: u32,
    pub data: Vec<u8>,
}

/// Verified, reassembled snapshot ready to apply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletedSnapshot {
    pub snapshot_id: SnapshotId,
    pub epoch: Epoch,
    pub at_tick: Tick,
    pub state_fingerprint: StateFingerprint,
    pub envelope: SnapshotEnvelope,
}

/// Host-side snapshot compressor and chunker.
#[derive(Debug, Clone)]
pub struct SnapshotSender {
    header: SnapshotHeader,
    compressed: Vec<u8>,
}

impl SnapshotSender {
    /// Compresses and prepares `envelope` for chunked transfer.
    pub fn from_envelope(
        snapshot_id: SnapshotId,
        epoch: Epoch,
        at_tick: Tick,
        state_fingerprint: StateFingerprint,
        envelope: &SnapshotEnvelope,
        compression: SnapshotCompression,
        chunk_size: u32,
    ) -> Result<Self, SnapshotError> {
        let raw = envelope.encode()?;
        Self::from_raw_bytes(
            snapshot_id,
            epoch,
            at_tick,
            state_fingerprint,
            &raw,
            compression,
            chunk_size,
        )
    }

    /// Compresses and prepares arbitrary raw payload bytes for chunked transfer.
    pub fn from_raw_bytes(
        snapshot_id: SnapshotId,
        epoch: Epoch,
        at_tick: Tick,
        state_fingerprint: StateFingerprint,
        raw_bytes: &[u8],
        compression: SnapshotCompression,
        chunk_size: u32,
    ) -> Result<Self, SnapshotError> {
        if chunk_size == 0 || chunk_size > MAX_SNAPSHOT_CHUNK_BYTES {
            return Err(SnapshotError::InvalidChunkSize {
                chunk_size,
                max: MAX_SNAPSHOT_CHUNK_BYTES,
            });
        }
        let uncompressed_len = raw_bytes.len() as u64;
        if uncompressed_len > MAX_SNAPSHOT_UNCOMPRESSED_BYTES {
            return Err(SnapshotError::UncompressedTooLarge {
                len: uncompressed_len,
                max: MAX_SNAPSHOT_UNCOMPRESSED_BYTES,
            });
        }

        let uncompressed_sha256 = sha256_bytes(raw_bytes);
        let compressed = match compression {
            SnapshotCompression::None => raw_bytes.to_vec(),
            SnapshotCompression::Deflate => {
                let mut encoder = DeflateEncoder::new(Vec::new(), Compression::fast());
                encoder
                    .write_all(raw_bytes)
                    .map_err(|e| SnapshotError::Compression(e.to_string()))?;
                encoder
                    .finish()
                    .map_err(|e| SnapshotError::Compression(e.to_string()))?
            }
        };

        let compressed_len = compressed.len() as u64;
        if compressed_len > MAX_SNAPSHOT_COMPRESSED_BYTES {
            return Err(SnapshotError::CompressedTooLarge {
                len: compressed_len,
                max: MAX_SNAPSHOT_COMPRESSED_BYTES,
            });
        }
        let compressed_sha256 = sha256_bytes(&compressed);
        let total_chunks = expected_chunk_count(compressed_len, chunk_size)?;

        let header = SnapshotHeader {
            snapshot_id,
            epoch,
            at_tick,
            state_fingerprint,
            compression,
            uncompressed_len,
            compressed_len,
            uncompressed_sha256,
            compressed_sha256,
            chunk_size,
            total_chunks,
        };
        Ok(Self { header, compressed })
    }

    /// Returns the snapshot header to transmit first.
    pub const fn header(&self) -> &SnapshotHeader {
        &self.header
    }

    /// Returns chunk `index`, or `None` if `index >= total_chunks`.
    pub fn chunk(&self, index: u32) -> Option<SnapshotChunk> {
        if index >= self.header.total_chunks {
            return None;
        }
        let chunk_size = self.header.chunk_size as usize;
        let start = (index as usize).checked_mul(chunk_size)?;
        let end = start.saturating_add(chunk_size).min(self.compressed.len());
        Some(SnapshotChunk {
            snapshot_id: self.header.snapshot_id,
            chunk_index: index,
            data: self.compressed[start..end].to_vec(),
        })
    }

    /// Returns all chunks in `0..total_chunks` order.
    pub fn chunks(&self) -> impl Iterator<Item = SnapshotChunk> + '_ {
        (0..self.header.total_chunks).filter_map(|i| self.chunk(i))
    }
}

/// Peer-side snapshot reassembler, bounds-checker, and SHA-256 verifier.
#[derive(Debug, Clone)]
pub struct SnapshotReceiver {
    header: SnapshotHeader,
    chunks: BTreeMap<u32, Vec<u8>>,
}

impl SnapshotReceiver {
    /// Validates `header` and creates a receiver ready for chunks.
    pub fn new(header: SnapshotHeader) -> Result<Self, SnapshotError> {
        if header.uncompressed_len > MAX_SNAPSHOT_UNCOMPRESSED_BYTES {
            return Err(SnapshotError::UncompressedTooLarge {
                len: header.uncompressed_len,
                max: MAX_SNAPSHOT_UNCOMPRESSED_BYTES,
            });
        }
        if header.compressed_len > MAX_SNAPSHOT_COMPRESSED_BYTES {
            return Err(SnapshotError::CompressedTooLarge {
                len: header.compressed_len,
                max: MAX_SNAPSHOT_COMPRESSED_BYTES,
            });
        }
        if header.chunk_size == 0 || header.chunk_size > MAX_SNAPSHOT_CHUNK_BYTES {
            return Err(SnapshotError::InvalidChunkSize {
                chunk_size: header.chunk_size,
                max: MAX_SNAPSHOT_CHUNK_BYTES,
            });
        }
        let expected = expected_chunk_count(header.compressed_len, header.chunk_size)?;
        if expected != header.total_chunks {
            return Err(SnapshotError::InconsistentChunkCount {
                expected,
                actual: header.total_chunks,
            });
        }
        Ok(Self {
            header,
            chunks: BTreeMap::new(),
        })
    }

    /// Returns the validated header.
    pub const fn header(&self) -> &SnapshotHeader {
        &self.header
    }

    /// Number of distinct chunks received so far.
    pub fn received_chunks(&self) -> u32 {
        self.chunks.len() as u32
    }

    /// Returns whether every expected chunk has been received.
    pub fn is_complete(&self) -> bool {
        self.received_chunks() == self.header.total_chunks
    }

    /// Accepts one chunk (in any order), checking bounds and deduplicating
    /// identical retransmissions.
    pub fn push_chunk(&mut self, chunk: SnapshotChunk) -> Result<bool, SnapshotError> {
        if chunk.snapshot_id != self.header.snapshot_id {
            return Err(SnapshotError::SnapshotIdMismatch {
                expected: self.header.snapshot_id,
                actual: chunk.snapshot_id,
            });
        }
        if chunk.chunk_index >= self.header.total_chunks {
            return Err(SnapshotError::ChunkOutOfRange {
                index: chunk.chunk_index,
                total_chunks: self.header.total_chunks,
            });
        }
        let expected_len = self.expected_chunk_len(chunk.chunk_index);
        if chunk.data.len() != expected_len {
            return Err(SnapshotError::ChunkSizeMismatch {
                index: chunk.chunk_index,
                expected: expected_len,
                actual: chunk.data.len(),
            });
        }
        if let Some(existing) = self.chunks.get(&chunk.chunk_index) {
            if existing != &chunk.data {
                return Err(SnapshotError::ConflictingDuplicateChunk {
                    index: chunk.chunk_index,
                });
            }
            return Ok(self.is_complete());
        }
        self.chunks.insert(chunk.chunk_index, chunk.data);
        Ok(self.is_complete())
    }

    /// Reassembles, verifies SHA-256 digests, and decompresses the raw bytes.
    pub fn finish_raw(self) -> Result<Vec<u8>, SnapshotError> {
        let received = self.received_chunks();
        if received != self.header.total_chunks {
            return Err(SnapshotError::Incomplete {
                received,
                total: self.header.total_chunks,
            });
        }

        let mut compressed = Vec::with_capacity(self.header.compressed_len as usize);
        for index in 0..self.header.total_chunks {
            let part = self.chunks.get(&index).ok_or(SnapshotError::Incomplete {
                received,
                total: self.header.total_chunks,
            })?;
            compressed.extend_from_slice(part);
        }

        if sha256_bytes(&compressed) != self.header.compressed_sha256 {
            return Err(SnapshotError::CompressedDigestMismatch);
        }

        let raw = match self.header.compression {
            SnapshotCompression::None => compressed,
            SnapshotCompression::Deflate => {
                // Bounded decompression prevents zip-bomb allocations.
                let limit = self.header.uncompressed_len.saturating_add(1);
                let mut decoder = DeflateDecoder::new(compressed.as_slice()).take(limit);
                let mut out = Vec::with_capacity(self.header.uncompressed_len as usize);
                decoder
                    .read_to_end(&mut out)
                    .map_err(|e| SnapshotError::Compression(e.to_string()))?;
                if (out.len() as u64) > self.header.uncompressed_len {
                    return Err(SnapshotError::UncompressedLengthMismatch {
                        expected: self.header.uncompressed_len,
                        actual: out.len() as u64,
                    });
                }
                out
            }
        };

        if raw.len() as u64 != self.header.uncompressed_len {
            return Err(SnapshotError::UncompressedLengthMismatch {
                expected: self.header.uncompressed_len,
                actual: raw.len() as u64,
            });
        }

        if sha256_bytes(&raw) != self.header.uncompressed_sha256 {
            return Err(SnapshotError::UncompressedDigestMismatch);
        }

        Ok(raw)
    }

    /// Reassembles, verifies, decompresses, and decodes the [`SnapshotEnvelope`].
    pub fn finish_envelope(self) -> Result<CompletedSnapshot, SnapshotError> {
        let snapshot_id = self.header.snapshot_id;
        let epoch = self.header.epoch;
        let at_tick = self.header.at_tick;
        let state_fingerprint = self.header.state_fingerprint;
        let raw = self.finish_raw()?;
        let envelope = SnapshotEnvelope::decode(&raw)?;
        Ok(CompletedSnapshot {
            snapshot_id,
            epoch,
            at_tick,
            state_fingerprint,
            envelope,
        })
    }

    fn expected_chunk_len(&self, index: u32) -> usize {
        let chunk_size = u64::from(self.header.chunk_size);
        let start = u64::from(index) * chunk_size;
        let remaining = self.header.compressed_len.saturating_sub(start);
        remaining.min(chunk_size) as usize
    }
}

fn expected_chunk_count(compressed_len: u64, chunk_size: u32) -> Result<u32, SnapshotError> {
    if compressed_len == 0 {
        return Ok(0);
    }
    let count = compressed_len.div_ceil(u64::from(chunk_size));
    let count_u32 = u32::try_from(count).map_err(|_| SnapshotError::TooManyChunks {
        count: u32::MAX,
        max: MAX_SNAPSHOT_CHUNKS,
    })?;
    if count_u32 > MAX_SNAPSHOT_CHUNKS {
        return Err(SnapshotError::TooManyChunks {
            count: count_u32,
            max: MAX_SNAPSHOT_CHUNKS,
        });
    }
    Ok(count_u32)
}
