//! Fixed-block memory pool allocator for the sandbox linear heap.
//!
//! The pool carves a hard-capped arena into equal-sized blocks. The arena
//! ceiling is always **≤ [`crate::MEM_LIMIT`] (10 MiB)** — callers may pass a
//! smaller limit for tests, but never a larger one.
//!
//! Allocations request whole blocks (size is rounded up). Exhaustion yields a
//! deterministic out-of-memory error so every mesh node observes the same fault.

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
    /// Requested `mem_limit` is zero or exceeds the global [`MEM_LIMIT`].
    LimitExceedsGlobal,
}

impl PoolError {
    pub fn as_str(self) -> &'static str {
        match self {
            PoolError::OutOfMemory => "Allocation failed: 10MB memory limit exceeded.",
            PoolError::InvalidBlockSize => "Invalid block size for fixed-block pool.",
            PoolError::ZeroSize => "Allocation size must be greater than zero.",
            PoolError::InvalidHandle => "Invalid block handle.",
            PoolError::LimitExceedsGlobal => {
                "Pool mem_limit must be > 0 and ≤ global MEM_LIMIT (10 MiB)."
            }
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

/// Fixed-block memory pool with a hard ceiling (≤ [`MEM_LIMIT`]).
///
/// Blocks are tracked with a free-list. Contiguous multi-block allocations use
/// first-fit over free runs so related tensor payloads stay packed.
///
/// **Invariant:** `capacity_bytes() ≤ mem_limit() ≤ MEM_LIMIT`. Growth past the
/// ceiling is impossible — `allocate` only marks free blocks; the arena is
/// pre-sized at construction.
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
    ///
    /// # Errors
    /// - [`PoolError::LimitExceedsGlobal`] if `mem_limit == 0` or `mem_limit > MEM_LIMIT`
    /// - [`PoolError::InvalidBlockSize`] if block size is zero or larger than the limit
    pub fn with_limit(block_size: usize, mem_limit: usize) -> Result<Self, PoolError> {
        // Hard global ceiling: no guest heap path may exceed MEM_LIMIT.
        if mem_limit == 0 || mem_limit > MEM_LIMIT {
            return Err(PoolError::LimitExceedsGlobal);
        }
        if block_size == 0 || block_size > mem_limit {
            return Err(PoolError::InvalidBlockSize);
        }
        let total_blocks = mem_limit / block_size;
        if total_blocks == 0 {
            return Err(PoolError::InvalidBlockSize);
        }
        // Arena is floor-aligned to block_size; never larger than mem_limit.
        let arena_bytes = total_blocks * block_size;
        debug_assert!(arena_bytes <= mem_limit);
        debug_assert!(arena_bytes <= MEM_LIMIT);
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
        // Fast path: not enough free blocks in aggregate (also covers need >
        // total_blocks, which would exceed capacity / MEM_LIMIT-bound arena).
        if need > self.free_blocks() {
            return Err(PoolError::OutOfMemory);
        }
        // Belt-and-braces: refuse if the would-be used byte count exceeds limit.
        let would_use = (self.used_blocks + need).saturating_mul(self.block_size);
        if would_use > self.mem_limit || would_use > MEM_LIMIT {
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
                    debug_assert!(self.used_bytes() <= self.mem_limit);
                    debug_assert!(self.used_bytes() <= MEM_LIMIT);
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
        assert!(pool.capacity_bytes() <= MEM_LIMIT);
    }

    #[test]
    fn oom_when_exceeding_limit() {
        let mut pool = FixedBlockPool::with_limit(64, 128).unwrap();
        let _a = pool.allocate(64).unwrap();
        let _b = pool.allocate(64).unwrap();
        let err = pool.allocate(1).unwrap_err();
        assert_eq!(err, PoolError::OutOfMemory);
    }

    #[test]
    fn rejects_limit_above_global_mem_limit() {
        let err = FixedBlockPool::with_limit(4096, MEM_LIMIT + 1).unwrap_err();
        assert_eq!(err, PoolError::LimitExceedsGlobal);
    }

    #[test]
    fn rejects_zero_mem_limit() {
        let err = FixedBlockPool::with_limit(64, 0).unwrap_err();
        assert_eq!(err, PoolError::LimitExceedsGlobal);
    }

    #[test]
    fn default_pool_capacity_is_mem_limit_aligned() {
        let pool = FixedBlockPool::new(4096).unwrap();
        assert_eq!(pool.mem_limit(), MEM_LIMIT);
        assert!(pool.capacity_bytes() <= MEM_LIMIT);
        assert_eq!(pool.capacity_bytes() % 4096, 0);
        // Full 10 MiB is divisible by 4 KiB.
        assert_eq!(pool.capacity_bytes(), MEM_LIMIT);
    }

    #[test]
    fn free_returns_capacity_for_reuse() {
        let mut pool = FixedBlockPool::with_limit(32, 64).unwrap();
        let a = pool.allocate(32).unwrap();
        let b = pool.allocate(32).unwrap();
        assert!(pool.allocate(1).is_err());
        pool.free(a).unwrap();
        let c = pool.allocate(32).unwrap();
        assert_eq!(c.start, 0);
        pool.free(b).unwrap();
        pool.free(c).unwrap();
        assert_eq!(pool.used_bytes(), 0);
    }

    #[test]
    fn multi_block_request_oom_when_fragmented_or_too_large() {
        let mut pool = FixedBlockPool::with_limit(64, 256).unwrap();
        // Ask for more than the entire arena.
        assert_eq!(
            pool.allocate(257).unwrap_err(),
            PoolError::OutOfMemory
        );
        // Fill with single blocks then free middle — first-fit still works for 1 block.
        let _h0 = pool.allocate(64).unwrap();
        let h1 = pool.allocate(64).unwrap();
        let _h2 = pool.allocate(64).unwrap();
        pool.free(h1).unwrap();
        // Contiguous 3-block request cannot fit free run of 1.
        assert_eq!(pool.allocate(192).unwrap_err(), PoolError::OutOfMemory);
        // 1-block request reuses the hole.
        assert!(pool.allocate(64).is_ok());
    }
}
