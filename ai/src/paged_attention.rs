//! # Edge PagedAttention
//!
//! Page table for disk-mapped / memory-backed context windows.
//!
//! Simulates vLLM-style paged KV cache management **without GPU drivers**:
//!
//! - Logical token slots map to fixed-size physical pages
//! - Pages may live in RAM ([`PageTable::new_memory`]) or on a file
//!   ([`PageTable::new_file`] — disk-mapped *simulation*, not OS `mmap`)
//! - Eviction is **deterministic LRU** by a monotonic generation counter
//!   (no wall-clock; same op sequence → same victim)
//!
//! Fully offline; no network.
//!
//! ## Public API
//!
//! ### High-level: [`PagedAttention`]
//!
//! Token-oriented helper (one page per token for stable addressing):
//!
//! 1. [`PagedAttention::new_memory`] / [`PagedAttention::new_file`]
//! 2. [`PagedAttention::append_token`] or [`PagedAttention::append_tensor`]
//! 3. [`PagedAttention::gather`] / [`PagedAttention::gather_fp32_tensor`]
//! 4. Optional [`PagedAttention::clear`]
//!
//! ### Low-level: [`PageTable`]
//!
//! Slot → physical page map with explicit lifecycle:
//!
//! - [`PageTable::map_slot`] / [`PageTable::unmap_slot`]
//! - [`PageTable::write_slot`] / [`PageTable::read_slot`]
//! - Inspect: [`PageTable::snapshot`], [`PageTable::mapped_count`], [`PageTable::backend_kind`]
//!
//! ### Types
//!
//! - [`PageId`] — opaque physical page handle
//! - [`PageBackendKind`] — `Memory` vs `File`
//! - [`PagedAttentionError`] — full / missing page / bad slot / I/O / SITF

use std::collections::HashMap;
use std::fmt;
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use crate::sitf::{DType, SitfError, SitfTensor};

/// Opaque physical page identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PageId(pub u32);

/// Errors for page table / paged attention operations.
#[derive(Debug)]
pub enum PagedAttentionError {
    /// No free physical pages and eviction could not satisfy the request,
    /// or the logical context window is full ([`PagedAttention::append_token`]).
    Full,
    /// Referenced page is not present in the in-memory pool.
    MissingPage(PageId),
    /// Logical slot index is outside the configured capacity.
    BadSlot { slot: usize, capacity: usize },
    /// File-backed I/O failure.
    Io(std::io::Error),
    /// Nested SITF construction / shape error.
    Sitf(SitfError),
    /// `page_size` / token byte length is zero or mismatched write length.
    InvalidPageSize,
}

impl fmt::Display for PagedAttentionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PagedAttentionError::Full => write!(f, "page table full: no free pages"),
            PagedAttentionError::MissingPage(id) => write!(f, "missing page {:?}", id),
            PagedAttentionError::BadSlot { slot, capacity } => {
                write!(f, "slot {slot} out of range (capacity {capacity})")
            }
            PagedAttentionError::Io(e) => write!(f, "io error: {e}"),
            PagedAttentionError::Sitf(e) => write!(f, "sitf error: {e}"),
            PagedAttentionError::InvalidPageSize => write!(f, "page size must be > 0"),
        }
    }
}

impl std::error::Error for PagedAttentionError {}

impl From<std::io::Error> for PagedAttentionError {
    fn from(e: std::io::Error) -> Self {
        PagedAttentionError::Io(e)
    }
}

impl From<SitfError> for PagedAttentionError {
    fn from(e: SitfError) -> Self {
        PagedAttentionError::Sitf(e)
    }
}

/// Where physical page bytes are stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageBackendKind {
    /// In-process RAM pages (fast path / tests).
    Memory,
    /// File-backed pages (simulates disk-mapped context window).
    File,
}

/// One physical page of KV / context data.
#[derive(Debug, Clone)]
struct PhysicalPage {
    id: PageId,
    /// Fixed-size payload (page_size bytes).
    data: Vec<u8>,
    /// Monotonic generation of last access (deterministic LRU).
    last_used: u64,
    /// Whether this page is currently mapped into a logical slot.
    in_use: bool,
}

/// Page table mapping logical context slots → physical pages.
///
/// Logical layout: `capacity_slots` contiguous token/context slots.
/// Each slot holds exactly one page of `page_size` bytes.
///
/// When the physical pool (`max_physical_pages`) is smaller than the logical
/// window, [`PageTable::map_slot`] / writes may evict the least-recently-used
/// page (lowest `last_used` generation) and clear its slot mapping.
#[derive(Debug)]
pub struct PageTable {
    page_size: usize,
    capacity_slots: usize,
    /// slot → PageId
    table: Vec<Option<PageId>>,
    pages: HashMap<PageId, PhysicalPage>,
    free_list: Vec<PageId>,
    next_page_id: u32,
    max_physical_pages: usize,
    clock: u64,
    backend: PageBackendKind,
    /// Optional file path for disk-mapped simulation.
    file_path: Option<PathBuf>,
}

impl PageTable {
    /// Create an in-memory page table.
    ///
    /// - `page_size`: bytes per page (must be > 0)
    /// - `capacity_slots`: logical context window length in pages/slots
    /// - `max_physical_pages`: physical pool size (enables oversubscription + eviction)
    pub fn new_memory(
        page_size: usize,
        capacity_slots: usize,
        max_physical_pages: usize,
    ) -> Result<Self, PagedAttentionError> {
        if page_size == 0 {
            return Err(PagedAttentionError::InvalidPageSize);
        }
        Ok(Self {
            page_size,
            capacity_slots,
            table: vec![None; capacity_slots],
            pages: HashMap::new(),
            free_list: Vec::new(),
            next_page_id: 0,
            max_physical_pages: max_physical_pages.max(1),
            clock: 0,
            backend: PageBackendKind::Memory,
            file_path: None,
        })
    }

    /// Create a file-backed page table.
    ///
    /// Pages are written/read at fixed offsets in `path` (simulates mmap'd
    /// context without requiring OS-specific APIs on the logical path).
    pub fn new_file(
        path: impl AsRef<Path>,
        page_size: usize,
        capacity_slots: usize,
        max_physical_pages: usize,
    ) -> Result<Self, PagedAttentionError> {
        if page_size == 0 {
            return Err(PagedAttentionError::InvalidPageSize);
        }
        let path = path.as_ref().to_path_buf();
        // Pre-size file to full logical window so offsets are stable.
        let total = page_size
            .checked_mul(capacity_slots)
            .ok_or(PagedAttentionError::InvalidPageSize)?;
        let f = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&path)?;
        f.set_len(total as u64)?;
        Ok(Self {
            page_size,
            capacity_slots,
            table: vec![None; capacity_slots],
            pages: HashMap::new(),
            free_list: Vec::new(),
            next_page_id: 0,
            max_physical_pages: max_physical_pages.max(1),
            clock: 0,
            backend: PageBackendKind::File,
            file_path: Some(path),
        })
    }

    /// Bytes per physical page.
    pub fn page_size(&self) -> usize {
        self.page_size
    }

    /// Number of logical slots in the context window.
    pub fn capacity_slots(&self) -> usize {
        self.capacity_slots
    }

    /// Memory vs file storage backend.
    pub fn backend_kind(&self) -> PageBackendKind {
        self.backend
    }

    /// Allocate (or reuse) a physical page and map it to `slot`.
    ///
    /// Returns the [`PageId`] assigned. Any existing mapping at `slot` is freed first.
    pub fn map_slot(&mut self, slot: usize) -> Result<PageId, PagedAttentionError> {
        if slot >= self.capacity_slots {
            return Err(PagedAttentionError::BadSlot {
                slot,
                capacity: self.capacity_slots,
            });
        }
        if let Some(old) = self.table[slot].take() {
            self.release_page(old);
        }
        let id = self.alloc_page()?;
        self.table[slot] = Some(id);
        if let Some(p) = self.pages.get_mut(&id) {
            p.in_use = true;
        }
        self.touch_by_id(id);
        Ok(id)
    }

    /// Write `data` (must be exactly `page_size`) into the page mapped at `slot`.
    ///
    /// Auto-maps the slot if it is currently unmapped.
    pub fn write_slot(&mut self, slot: usize, data: &[u8]) -> Result<(), PagedAttentionError> {
        if data.len() != self.page_size {
            return Err(PagedAttentionError::InvalidPageSize);
        }
        let id = match self.table.get(slot).and_then(|x| *x) {
            Some(id) => id,
            None => self.map_slot(slot)?,
        };
        {
            let page = self
                .pages
                .get_mut(&id)
                .ok_or(PagedAttentionError::MissingPage(id))?;
            page.data.copy_from_slice(data);
        }
        self.touch_by_id(id);
        if self.backend == PageBackendKind::File {
            self.persist_page(slot, id)?;
        }
        Ok(())
    }

    /// Read page bytes for `slot`. Touches LRU. Errors if unmapped.
    pub fn read_slot(&mut self, slot: usize) -> Result<Vec<u8>, PagedAttentionError> {
        if slot >= self.capacity_slots {
            return Err(PagedAttentionError::BadSlot {
                slot,
                capacity: self.capacity_slots,
            });
        }
        let id = self.table[slot].ok_or(PagedAttentionError::BadSlot {
            slot,
            capacity: self.capacity_slots,
        })?;
        // If page was evicted from RAM but is file-backed, reload.
        if !self.pages.contains_key(&id) {
            if self.backend == PageBackendKind::File {
                self.reload_page(slot, id)?;
            } else {
                return Err(PagedAttentionError::MissingPage(id));
            }
        }
        self.touch_by_id(id);
        let page = self
            .pages
            .get(&id)
            .ok_or(PagedAttentionError::MissingPage(id))?;
        Ok(page.data.clone())
    }

    /// Unmap a logical slot and return its page to the free list.
    pub fn unmap_slot(&mut self, slot: usize) -> Result<(), PagedAttentionError> {
        if slot >= self.capacity_slots {
            return Err(PagedAttentionError::BadSlot {
                slot,
                capacity: self.capacity_slots,
            });
        }
        if let Some(id) = self.table[slot].take() {
            self.release_page(id);
        }
        Ok(())
    }

    /// Number of currently mapped logical slots.
    pub fn mapped_count(&self) -> usize {
        self.table.iter().filter(|s| s.is_some()).count()
    }

    /// Snapshot of the page table (slot → PageId) for deterministic inspection.
    pub fn snapshot(&self) -> Vec<Option<PageId>> {
        self.table.clone()
    }

    fn alloc_page(&mut self) -> Result<PageId, PagedAttentionError> {
        if let Some(id) = self.free_list.pop() {
            if let Some(p) = self.pages.get_mut(&id) {
                p.data.fill(0);
                p.in_use = true;
            }
            return Ok(id);
        }
        if self.pages.len() < self.max_physical_pages {
            let id = PageId(self.next_page_id);
            self.next_page_id = self.next_page_id.saturating_add(1);
            self.pages.insert(
                id,
                PhysicalPage {
                    id,
                    data: vec![0u8; self.page_size],
                    last_used: 0,
                    in_use: true,
                },
            );
            return Ok(id);
        }
        // Evict deterministic LRU among physical pages, detach from slots, reuse.
        let victim = self
            .pages
            .values()
            .min_by_key(|p| p.last_used)
            .map(|p| p.id)
            .ok_or(PagedAttentionError::Full)?;
        for slot in self.table.iter_mut() {
            if *slot == Some(victim) {
                *slot = None;
            }
        }
        if let Some(p) = self.pages.get_mut(&victim) {
            p.data.fill(0);
            p.in_use = true;
        }
        Ok(victim)
    }

    fn release_page(&mut self, id: PageId) {
        if let Some(p) = self.pages.get_mut(&id) {
            p.in_use = false;
            p.data.fill(0);
        }
        if !self.free_list.contains(&id) {
            self.free_list.push(id);
        }
    }

    fn touch_by_id(&mut self, id: PageId) {
        self.clock = self.clock.saturating_add(1);
        let c = self.clock;
        if let Some(p) = self.pages.get_mut(&id) {
            p.last_used = c;
        }
    }

    fn persist_page(&self, slot: usize, id: PageId) -> Result<(), PagedAttentionError> {
        let path = self.file_path.as_ref().ok_or_else(|| {
            PagedAttentionError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "no file path",
            ))
        })?;
        let page = self
            .pages
            .get(&id)
            .ok_or(PagedAttentionError::MissingPage(id))?;
        let mut f = OpenOptions::new().read(true).write(true).open(path)?;
        let offset = (slot * self.page_size) as u64;
        f.seek(SeekFrom::Start(offset))?;
        f.write_all(&page.data)?;
        f.flush()?;
        Ok(())
    }

    fn reload_page(&mut self, slot: usize, id: PageId) -> Result<(), PagedAttentionError> {
        let path = self
            .file_path
            .as_ref()
            .ok_or_else(|| {
                PagedAttentionError::Io(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "no file path",
                ))
            })?
            .clone();
        // Ensure capacity for re-insert
        if !self.pages.contains_key(&id) && self.pages.len() >= self.max_physical_pages {
            let _ = self.alloc_page()?;
            if let Some(extra) = self.free_list.pop() {
                self.pages.remove(&extra);
            }
        }
        let mut f = File::open(&path)?;
        let offset = (slot * self.page_size) as u64;
        f.seek(SeekFrom::Start(offset))?;
        let mut data = vec![0u8; self.page_size];
        f.read_exact(&mut data)?;
        self.pages.insert(
            id,
            PhysicalPage {
                id,
                data,
                last_used: 0,
                in_use: true,
            },
        );
        Ok(())
    }
}

/// High-level Edge PagedAttention helper: stores per-token key/value slices
/// in the page table and gathers a contiguous context window.
///
/// One token occupies one page (`token_bytes == page_size`) for simple,
/// stable addressing suitable for edge offline replay.
#[derive(Debug)]
pub struct PagedAttention {
    table: PageTable,
    /// Bytes per token KV blob (one token per page for stable addressing).
    token_bytes: usize,
    /// Next free logical token index in the window.
    next_token: usize,
    /// Max tokens in the context window.
    max_tokens: usize,
}

impl PagedAttention {
    /// Create a memory-backed paged attention context.
    ///
    /// `token_bytes` is the serialized KV size per token
    /// (e.g. `2 * head_dim * size_of::<f32>()`).
    pub fn new_memory(
        token_bytes: usize,
        max_tokens: usize,
        max_physical_pages: usize,
    ) -> Result<Self, PagedAttentionError> {
        if token_bytes == 0 {
            return Err(PagedAttentionError::InvalidPageSize);
        }
        // One token per page for simplicity and stable addressing.
        let page_size = token_bytes;
        let table = PageTable::new_memory(page_size, max_tokens, max_physical_pages)?;
        Ok(Self {
            table,
            token_bytes,
            next_token: 0,
            max_tokens,
        })
    }

    /// File-backed context window (disk-mapped simulation).
    pub fn new_file(
        path: impl AsRef<Path>,
        token_bytes: usize,
        max_tokens: usize,
        max_physical_pages: usize,
    ) -> Result<Self, PagedAttentionError> {
        if token_bytes == 0 {
            return Err(PagedAttentionError::InvalidPageSize);
        }
        let page_size = token_bytes;
        let table = PageTable::new_file(path, page_size, max_tokens, max_physical_pages)?;
        Ok(Self {
            table,
            token_bytes,
            next_token: 0,
            max_tokens,
        })
    }

    /// Number of tokens currently stored (`0..max_tokens`).
    pub fn len(&self) -> usize {
        self.next_token
    }

    /// Whether the context window has no tokens.
    pub fn is_empty(&self) -> bool {
        self.next_token == 0
    }

    /// Configured maximum token count for this window.
    pub fn max_tokens(&self) -> usize {
        self.max_tokens
    }

    /// Borrow the underlying page table (inspection / advanced control).
    pub fn page_table(&self) -> &PageTable {
        &self.table
    }

    /// Append a token's KV bytes to the context. Returns the token index.
    ///
    /// Errors if `kv.len() != token_bytes` or the window is full.
    pub fn append_token(&mut self, kv: &[u8]) -> Result<usize, PagedAttentionError> {
        if kv.len() != self.token_bytes {
            return Err(PagedAttentionError::InvalidPageSize);
        }
        if self.next_token >= self.max_tokens {
            return Err(PagedAttentionError::Full);
        }
        let idx = self.next_token;
        self.table.write_slot(idx, kv)?;
        self.next_token += 1;
        Ok(idx)
    }

    /// Append from an INT8 or FP32 SITF tensor (raw payload used as KV bytes).
    ///
    /// Payload length must equal `token_bytes`.
    pub fn append_tensor(&mut self, t: &SitfTensor) -> Result<usize, PagedAttentionError> {
        if t.data.len() != self.token_bytes {
            return Err(PagedAttentionError::InvalidPageSize);
        }
        self.append_token(&t.data)
    }

    /// Gather tokens `[start, end)` into a single contiguous byte buffer.
    pub fn gather(&mut self, start: usize, end: usize) -> Result<Vec<u8>, PagedAttentionError> {
        if end > self.next_token || start > end {
            return Err(PagedAttentionError::BadSlot {
                slot: end,
                capacity: self.next_token,
            });
        }
        let mut out = Vec::with_capacity((end - start) * self.token_bytes);
        for i in start..end {
            out.extend_from_slice(&self.table.read_slot(i)?);
        }
        Ok(out)
    }

    /// Gather as a rank-2 SITF FP32 tensor if payload length is a multiple of 4:
    /// shape = `[num_tokens, floats_per_token]`.
    pub fn gather_fp32_tensor(
        &mut self,
        start: usize,
        end: usize,
    ) -> Result<SitfTensor, PagedAttentionError> {
        let bytes = self.gather(start, end)?;
        if bytes.len() % 4 != 0 {
            return Err(PagedAttentionError::Sitf(SitfError::ShapeMismatch {
                expected_elems: bytes.len() / 4,
                data_len: bytes.len(),
            }));
        }
        let n_tokens = end - start;
        let floats_per = self.token_bytes / 4;
        if self.token_bytes % 4 != 0 {
            return Err(PagedAttentionError::InvalidPageSize);
        }
        SitfTensor::new(
            vec![n_tokens as u32, floats_per as u32],
            DType::Fp32,
            bytes,
        )
        .map_err(Into::into)
    }

    /// Clear the context window (unmaps all used slots).
    pub fn clear(&mut self) -> Result<(), PagedAttentionError> {
        for i in 0..self.next_token {
            self.table.unmap_slot(i)?;
        }
        self.next_token = 0;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn memory_page_table_write_read() {
        let mut pt = PageTable::new_memory(8, 4, 4).unwrap();
        let data = [1u8, 2, 3, 4, 5, 6, 7, 8];
        pt.write_slot(0, &data).unwrap();
        pt.write_slot(2, &[9, 9, 9, 9, 9, 9, 9, 9]).unwrap();
        assert_eq!(pt.read_slot(0).unwrap(), data);
        assert_eq!(pt.read_slot(2).unwrap()[0], 9);
        assert_eq!(pt.mapped_count(), 2);
    }

    #[test]
    fn paged_attention_append_gather() {
        let mut pa = PagedAttention::new_memory(4, 8, 8).unwrap();
        pa.append_token(&[1, 0, 0, 0]).unwrap();
        pa.append_token(&[2, 0, 0, 0]).unwrap();
        pa.append_token(&[3, 0, 0, 0]).unwrap();
        let g = pa.gather(0, 3).unwrap();
        assert_eq!(g, vec![1, 0, 0, 0, 2, 0, 0, 0, 3, 0, 0, 0]);
    }

    #[test]
    fn file_backed_context() {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("mossymesh_ai_page_{nanos}.bin"));
        let mut pa = PagedAttention::new_file(&path, 4, 4, 2).unwrap();
        pa.append_token(&[10, 20, 30, 40]).unwrap();
        pa.append_token(&[50, 60, 70, 80]).unwrap();
        let g = pa.gather(0, 2).unwrap();
        assert_eq!(&g[..4], &[10, 20, 30, 40]);
        assert_eq!(&g[4..], &[50, 60, 70, 80]);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn eviction_under_pressure() {
        // 4 slots, only 2 physical pages → older pages get unmapped from table on reuse
        let mut pt = PageTable::new_memory(4, 4, 2).unwrap();
        pt.write_slot(0, &[1, 1, 1, 1]).unwrap();
        pt.write_slot(1, &[2, 2, 2, 2]).unwrap();
        // Third allocation forces eviction of LRU (slot 0)
        pt.write_slot(2, &[3, 3, 3, 3]).unwrap();
        assert_eq!(pt.mapped_count(), 2);
        assert!(pt.read_slot(2).is_ok());
    }

    #[test]
    fn append_tensor_fp32() {
        let mut pa = PagedAttention::new_memory(8, 4, 4).unwrap();
        let t = SitfTensor::from_f32(vec![2], &[1.5, -2.5]).unwrap();
        pa.append_tensor(&t).unwrap();
        let out = pa.gather_fp32_tensor(0, 1).unwrap();
        let vals = out.as_f32_vec().unwrap();
        assert_eq!(vals, vec![1.5, -2.5]);
    }

    /// Same append sequence always gathers the same bytes (deterministic context).
    #[test]
    fn gather_deterministic_across_runs() {
        let kv = [
            [1u8, 2, 3, 4],
            [5, 6, 7, 8],
            [9, 10, 11, 12],
        ];
        let mut g1 = {
            let mut pa = PagedAttention::new_memory(4, 8, 8).unwrap();
            for row in &kv {
                pa.append_token(row).unwrap();
            }
            pa.gather(0, 3).unwrap()
        };
        let g2 = {
            let mut pa = PagedAttention::new_memory(4, 8, 8).unwrap();
            for row in &kv {
                pa.append_token(row).unwrap();
            }
            pa.gather(0, 3).unwrap()
        };
        assert_eq!(g1, g2);
        g1.clear(); // ensure we didn't alias
        assert_eq!(g2.len(), 12);
    }

    #[test]
    fn snapshot_maps_slots_deterministically() {
        let mut pt = PageTable::new_memory(4, 3, 3).unwrap();
        let id0 = pt.map_slot(0).unwrap();
        let id1 = pt.map_slot(1).unwrap();
        let snap = pt.snapshot();
        assert_eq!(snap[0], Some(id0));
        assert_eq!(snap[1], Some(id1));
        assert_eq!(snap[2], None);
        assert_eq!(pt.backend_kind(), PageBackendKind::Memory);
    }
}
