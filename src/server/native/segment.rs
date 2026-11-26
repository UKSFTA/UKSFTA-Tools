//! Split one file into disjoint byte ranges.

use crate::error::UksftaError;

/// The smallest chunk.
pub const MIN_CHUNK: u32 = 1024 * 1024;
/// The largest chunk a caller may set.
pub const MAX_CHUNK: u32 = 64 * 1024 * 1024;
/// The largest chunk the planner picks automatically.
pub const AUTO_MAX_CHUNK: u32 = 16 * 1024 * 1024;

/// One range of the file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Chunk {
    pub index: u32,
    pub start: u64,
    pub len: u32,
}

/// Every range of one file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SegmentPlan {
    pub size: u64,
    pub chunk: u32,
    pub chunks: Vec<Chunk>,
}

/// A chunk for `size` bytes across `connections` workers, clamped to
/// 1 MiB..=16 MiB and rounded down to a 4096-byte multiple.
pub fn auto_chunk(size: u64, connections: u8) -> u32 {
    let divisor = u64::from(connections.max(1)) * 8;
    let raw = size / divisor;
    let clamped = raw.clamp(u64::from(MIN_CHUNK), u64::from(AUTO_MAX_CHUNK));
    let rounded = (clamped / 4096) * 4096;
    u32::try_from(rounded.max(4096)).unwrap_or(MIN_CHUNK)
}

impl SegmentPlan {
    /// # Errors
    /// Returns `Input` for an empty file, a chunk outside the range, or a
    /// chunk that is not a 4096-byte multiple.
    pub fn new(size: u64, connections: u8, chunk: Option<u32>) -> Result<Self, UksftaError> {
        if size == 0 {
            return Err(UksftaError::Input(
                "an empty file uses the whole-file path".to_string(),
            ));
        }
        let chunk = match chunk {
            Some(chunk) => {
                if !(MIN_CHUNK..=MAX_CHUNK).contains(&chunk) || chunk % 4096 != 0 {
                    return Err(UksftaError::Input(format!(
                        "segment chunk must be {MIN_CHUNK}..={MAX_CHUNK} bytes and a 4096-byte multiple: {chunk}"
                    )));
                }
                chunk
            }
            None => auto_chunk(size, connections),
        };

        let mut chunks = Vec::new();
        let mut start = 0u64;
        while start < size {
            let len = (size - start).min(u64::from(chunk));
            chunks.push(Chunk {
                index: u32::try_from(chunks.len())
                    .map_err(|_| UksftaError::Input("too many ranges".to_string()))?,
                start,
                len: u32::try_from(len)
                    .map_err(|_| UksftaError::Input(format!("segment range is too long: {len}")))?,
            });
            start += len;
        }
        Ok(Self {
            size,
            chunk,
            chunks,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIB: u64 = 1024 * 1024;

    #[test]
    fn the_automatic_chunk_is_clamped_and_rounded() {
        assert_eq!(auto_chunk(1, 4), MIN_CHUNK);
        assert_eq!(auto_chunk(977 * MIB, 4), AUTO_MAX_CHUNK);
        assert_eq!(auto_chunk(32 * MIB, 16), MIN_CHUNK);
        assert_eq!(auto_chunk(5 * 1024 * MIB + 1, 1), AUTO_MAX_CHUNK);
    }

    #[test]
    fn the_ranges_cover_the_file_without_a_gap_or_overlap() {
        for size in [1, 4095, MIB, 977 * MIB + 13, 5 * 1024 * MIB + 1] {
            for connections in [1, 4, 16] {
                for chunk in [None, Some(MIN_CHUNK), Some(MAX_CHUNK)] {
                    let plan = SegmentPlan::new(size, connections, chunk).unwrap();
                    let total: u64 = plan.chunks.iter().map(|c| u64::from(c.len)).sum();
                    assert_eq!(total, size, "{size} {connections} {chunk:?}");

                    let mut expected_start = 0;
                    for (index, range) in plan.chunks.iter().enumerate() {
                        assert_eq!(range.start, expected_start);
                        assert_eq!(range.index as usize, index);
                        assert!(u64::from(range.len) <= u64::from(plan.chunk));
                        expected_start += u64::from(range.len);
                    }
                    for range in plan.chunks.iter().take(plan.chunks.len() - 1) {
                        assert_eq!(range.len, plan.chunk);
                    }
                    assert!(u64::from(plan.chunks.last().unwrap().len) <= u64::from(plan.chunk));
                }
            }
        }
    }

    #[test]
    fn a_large_file_yields_the_expected_plan() {
        let plan = SegmentPlan::new(977 * MIB, 4, None).unwrap();
        assert_eq!(plan.chunk, AUTO_MAX_CHUNK);
        assert_eq!(plan.chunks.len(), 62);
    }

    #[test]
    fn bad_plans_are_refused() {
        assert!(SegmentPlan::new(0, 4, None).is_err());
        assert!(SegmentPlan::new(MIB, 4, Some(MIN_CHUNK + 1)).is_err());
        assert!(SegmentPlan::new(MIB, 4, Some(MAX_CHUNK + 4096)).is_err());
    }
}
