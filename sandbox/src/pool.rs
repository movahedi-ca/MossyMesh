//! Fixed-block memory pool allocator for the sandbox linear heap.
//!
//! The pool carves a hard-capped arena (`MEM_LIMIT` by default) into
//! equal-sized blocks. Allocations request whole blocks (size is rounded up).
//! Exhaustion yields a deterministic out-of-memory error so every mesh node
//! observes the same fault.

use crate::MEM_LIMIT;

/// Deterministic pool / allocation errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PoolError {
    /// Request would exceed the configured memory limit.
    OutOfMemory,
    /// Block size must be non-zero and not larger than the limit.
    InvalidBlockSize,
    /// Size argument was zero.
    ZeroSize,
    /// Handle does not refer to an allocated block.
    InvalidHandle,
}

impl PoolError {
    pub fn as_str(self) -> &'static str {
        match self {
            PoolError::OutOfMemory => "Allocation failed: 10MB memory limit exceeded.",
            PoolError::InvalidBlockSize => "Invalid block size for fixed-block pool.",
            PoolError::ZeroSize => "Allocation size must be greater than zero.",
            PoolError::InvalidHandle => "Invalid block handle.",
        }
    }
}

impl core::fmt::Display for PoolError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Opaque handle to one or more contiguous blocks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BlockHandle {
    /// Index of the first block in the arena.
    pub(crate) start: usize,
    /// Number of contiguous blocks owned by this handle.
    pub(crate) count: usize,
}

impl BlockHandle {
    /// Byte offset into the pool arena (guest-linear-memory style pointer).
    pub fn offset(&self, block_size: usize) -> usize {
        self.start.saturating_mul(block_size)
    }

    /// Total bytes covered by this handle.
    pub fn byte_len(&self, block_size: usize) -> usize {
        self.count.saturating_mul(block_size)
    }
}

/// Fixed-block memory pool with a hard ceiling.
///
/// Blocks are tracked with a free-list. Contiguous multi-block allocations use
/// first-fit over free runs so related tensor payloads stay packed.
#[derive(Debug)]
pub struct FixedBlockPool {
    block_size: usize,
    mem_limit: usize,
    /// Backing arena; length is always `total_blocks * block_size` once initialized.
    storage: Vec<u8>,
    /// `true` when block `i` is free.
    free: Vec<bool>,
    used_blocks: usize,
    total_blocks: usize,
}

impl FixedBlockPool {
    /// Create a pool with the given block size and the global [`MEM_LIMIT`].
    pub fn new(block_size: usize) -> Result<Self, PoolError> {
        Self::with_limit(block_size, MEM_LIMIT)
    }

    /// Create a pool with an explicit memory ceiling (useful for unit tests).
    pub fn with_limit(block_size: usize, mem_limit: usize) -> Result<Self, PoolError> {
        if block_size == 0 || block_size > mem_limit {
            return Err(PoolError::InvalidBlockSize);
        }
        let total_blocks = mem_limit / block_size;
        if total_blocks == 0 {
            return Err(PoolError::InvalidBlockSize);
        }
        let arena_bytes = total_blocks * block_size;
        Ok(Self {
            block_size,
            mem_limit,
            storage: vec![0u8; arena_bytes],
            free: vec![true; total_blocks],
            used_blocks: 0,
            total_blocks,
        })
    }

    pub fn block_size(&self) -> usize {
        self.block_size
    }

    pub fn mem_limit(&self) -> usize {
        self.mem_limit
    }

    pub fn total_blocks(&self) -> usize {
        self.total_blocks
    }

    pub fn used_blocks(&self) -> usize {
        self.used_blocks
    }

    pub fn free_blocks(&self) -> usize {
        self.total_blocks - self.used_blocks
    }

    /// Bytes currently held by live allocations (block-aligned).
    pub fn used_bytes(&self) -> usize {
        self.used_blocks * self.block_size
    }

    /// Usable arena capacity in bytes (may be slightly under `mem_limit` if
    /// `mem_limit` is not an exact multiple of `block_size`).
    pub fn capacity_bytes(&self) -> usize {
        self.total_blocks * self.block_size
    }

    /// Number of whole blocks required to cover `size` bytes.
    pub fn blocks_for(&self, size: usize) -> Result<usize, PoolError> {
        if size == 0 {
            return Err(PoolError::ZeroSize);
        }
        Ok(size.div_ceil(self.block_size))
    }

    /// Allocate enough contiguous blocks to cover `size` bytes.
    ///
    /// Returns [`PoolError::OutOfMemory`] when the request cannot be satisfied
    /// within the hard limit — never grows past the ceiling.
    pub fn allocate(&mut self, size: usize) -> Result<BlockHandle, PoolError> {
        let need = self.blocks_for(size)?;
        if need > self.free_blocks() {
            return Err(PoolError::OutOfMemory);
        }

        // First-fit over free runs.
        let mut i = 0usize;
        while i < self.total_blocks {
            if !self.free[i] {
                i += 1;
                continue;
            }
            let mut run = 0usize;
            while i + run < self.total_blocks && self.free[i + run] {
                run += 1;
                if run == need {
                    for b in i..i + need {
                        self.free[b] = false;
                    }
                    self.used_blocks += need;
                    return Ok(BlockHandle {
                        start: i,
                        count: need,
                    });
                }
            }
            i += run.max(1);
        }
        Err(PoolError::OutOfMemory)
    }

    /// Release a previously allocated handle back to the free list.
    pub fn free(&mut self, handle: BlockHandle) -> Result<(), PoolError> {
        if handle.count == 0
            || handle.start >= self.total_blocks
            || handle.start + handle.count > self.total_blocks
        {
            return Err(PoolError::InvalidHandle);
        }
        for b in handle.start..handle.start + handle.count {
            if self.free[b] {
                return Err(PoolError::InvalidHandle);
            }
            self.free[b] = true;
        }
        self.used_blocks -= handle.count;
        Ok(())
    }

    /// Read-only view of the arena bytes for a handle.
    pub fn get(&self, handle: BlockHandle) -> Result<&[u8], PoolError> {
        self.validate(handle)?;
        let start = handle.offset(self.block_size);
        let end = start + handle.byte_len(self.block_size);
        Ok(&self.storage[start..end])
    }

    /// Mutable view of the arena bytes for a handle.
    pub fn get_mut(&mut self, handle: BlockHandle) -> Result<&mut [u8], PoolError> {
        self.validate(handle)?;
        let start = handle.offset(self.block_size);
        let end = start + handle.byte_len(self.block_size);
        Ok(&mut self.storage[start..end])
    }

    /// Write payload into an allocated region (truncated/padded to the region).
    pub fn write(&mut self, handle: BlockHandle, data: &[u8]) -> Result<(), PoolError> {
        let region = self.get_mut(handle)?;
        let n = data.len().min(region.len());
        region[..n].copy_from_slice(&data[..n]);
        if n < region.len() {
            region[n..].fill(0);
        }
        Ok(())
    }

    fn validate(&self, handle: BlockHandle) -> Result<(), PoolError> {
        if handle.count == 0
            || handle.start >= self.total_blocks
            || handle.start + handle.count > self.total_blocks
        {
            return Err(PoolError::InvalidHandle);
        }
        for b in handle.start..handle.start + handle.count {
            if self.free[b] {
                return Err(PoolError::InvalidHandle);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allocates_under_limit() {
        let mut pool = FixedBlockPool::with_limit(64, 256).unwrap();
        let h = pool.allocate(100).unwrap();
        assert_eq!(h.count, 2); // 100 rounds up to 128
        assert_eq!(pool.used_bytes(), 128);
        assert!(pool.used_bytes() <= pool.capacity_bytes());
    }

    #[test]
    fn oom_when_exceeding_limit() {
        let mut pool = FixedBlockPool::with_limit(64, 128).unwrap();
        let _a = pool.allocate(64).unwrap();
        let _b = pool.allocate(64).unwrap();
        let err = pool.allocate(1).unwrap_err();
        assert_eq!(err, PoolError::OutOfMemory);
    }
}
