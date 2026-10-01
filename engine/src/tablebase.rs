//! Syzygy tablebase hook: trait + stub, optional real probing behind `syzygy` feature.
//!
//! When table files are absent, probes return `None` (caller falls back to eval/search).
//! File-backed mmap is available only with features `syzygy` / `syzygy-mmap` (native).
//! Default builds stay wasm32-wasip1 portable (no mmap APIs).

use shakmaty::Chess;

/// Coarse WDL-style result for mesh determinism.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TbWdl {
    Loss = -2,
    BlessedLoss = -1,
    Draw = 0,
    CursedWin = 1,
    Win = 2,
}

impl TbWdl {
    /// Map to a large centipawn-style score (for search integration).
    pub fn to_score(self) -> i32 {
        match self {
            TbWdl::Loss => -20_000,
            TbWdl::BlessedLoss => -10_000,
            TbWdl::Draw => 0,
            TbWdl::CursedWin => 10_000,
            TbWdl::Win => 20_000,
        }
    }
}

/// Portable tablebase probe interface (mesh / WASM safe).
pub trait TablebaseProbe {
    /// Whether any tables are loaded and usable.
    fn is_available(&self) -> bool;

    /// Directory path(s) configured (empty if stub / unloaded).
    fn paths(&self) -> &[String];

    /// Probe WDL for `pos`. `None` if not available, not in table, or error.
    fn probe_wdl(&self, pos: &Chess) -> Option<TbWdl>;
}

/// Always-unavailable stub used when tables are missing or feature is off.
#[derive(Debug, Default, Clone)]
pub struct StubTablebase {
    paths: Vec<String>,
}

impl StubTablebase {
    pub fn new() -> Self {
        Self { paths: Vec::new() }
    }

    /// Record intended table directory without loading (documents config for mesh nodes).
    pub fn with_path(path: impl Into<String>) -> Self {
        Self {
            paths: vec![path.into()],
        }
    }
}

impl TablebaseProbe for StubTablebase {
    fn is_available(&self) -> bool {
        false
    }

    fn paths(&self) -> &[String] {
        &self.paths
    }

    fn probe_wdl(&self, _pos: &Chess) -> Option<TbWdl> {
        None
    }
}

/// Deterministic 64-bit FNV-1a hash (stable across processes and nodes,
/// unlike the randomized DefaultHasher).
fn fnv1a_64(bytes: &[u8], mut h: u64) -> u64 {
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

fn collect_table_files(dir: &std::path::Path, out: &mut Vec<(String, u64)>) {
    let rd = match std::fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(_) => return,
    };
    for entry in rd.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_table_files(&path, out);
        } else if let Ok(md) = entry.metadata() {
            out.push((path.to_string_lossy().into_owned(), md.len()));
        }
    }
}

/// Fingerprint the table corpus from sorted (path, size) entries.
///
/// Nodes compare fingerprints before treating probe results as
/// deterministic across the mesh: a mismatch means the corpora differ and
/// outputs must not be compared. This does not prevent external mutation
/// after open (still caller responsibility), it makes divergence detectable.
fn fingerprint_corpus(paths: &[String]) -> u64 {
    let mut entries: Vec<(String, u64)> = Vec::new();
    for p in paths {
        collect_table_files(std::path::Path::new(p), &mut entries);
    }
    entries.sort();
    let mut h: u64 = 0xcbf29ce484222325;
    for (name, len) in &entries {
        h = fnv1a_64(name.as_bytes(), h);
        h = fnv1a_64(&len.to_le_bytes(), h);
    }
    h
}

/// File-backed tablebase handle.
///
/// - Without `syzygy` feature: behaves as a path-aware stub (no I/O, always miss).
/// - With `syzygy`: loads via `shakmaty_syzygy::Tablebase` when directories exist.
/// - With `syzygy-mmap`: prefers mmap filesystem when the dependency enables it.
pub struct FileBackedTablebase {
    paths: Vec<String>,
    /// Corpus fingerprint at open time; see [`fingerprint_corpus`].
    corpus_fingerprint: u64,
    #[cfg(feature = "syzygy")]
    inner: Option<shakmaty_syzygy::Tablebase<Chess>>,
}

impl std::fmt::Debug for FileBackedTablebase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FileBackedTablebase")
            .field("paths", &self.paths)
            .field("available", &self.is_available())
            .finish()
    }
}

impl FileBackedTablebase {
    pub fn empty() -> Self {
        Self {
            paths: Vec::new(),
            corpus_fingerprint: 0xcbf29ce484222325,
            #[cfg(feature = "syzygy")]
            inner: None,
        }
    }

    /// Corpus fingerprint captured at open time. Two nodes with different
    /// fingerprints hold different table files and must not treat probe
    /// results as mutually deterministic.
    pub fn corpus_fingerprint(&self) -> u64 {
        self.corpus_fingerprint
    }

    /// Open tablebase directories. If none can be opened (missing files), remains a stub-like miss.
    pub fn open(paths: impl IntoIterator<Item = impl Into<String>>) -> Self {
        let paths: Vec<String> = paths.into_iter().map(Into::into).collect();
        let corpus_fingerprint = fingerprint_corpus(&paths);

        #[cfg(feature = "syzygy")]
        {
            // Prefer mmap filesystem when feature enabled (native, 64-bit).
            #[cfg(feature = "syzygy-mmap")]
            let mut tb: shakmaty_syzygy::Tablebase<Chess> = {
                // SAFETY: caller must not mutate table files after open; mesh nodes treat
                // table dirs as read-only assets.
                unsafe { shakmaty_syzygy::Tablebase::with_mmap_filesystem() }
            };
            #[cfg(not(feature = "syzygy-mmap"))]
            let mut tb: shakmaty_syzygy::Tablebase<Chess> = shakmaty_syzygy::Tablebase::new();

            let mut any = false;
            for p in &paths {
                match tb.add_directory(p) {
                    Ok(n) if n > 0 => any = true,
                    Ok(_) => {}
                    Err(_) => {}
                }
            }
            return Self {
                paths,
                corpus_fingerprint,
                inner: if any { Some(tb) } else { None },
            };
        }

        #[cfg(not(feature = "syzygy"))]
        {
            Self {
                paths,
                corpus_fingerprint,
            }
        }
    }
}

impl TablebaseProbe for FileBackedTablebase {
    fn is_available(&self) -> bool {
        #[cfg(feature = "syzygy")]
        {
            self.inner.is_some()
        }
        #[cfg(not(feature = "syzygy"))]
        {
            false
        }
    }

    fn paths(&self) -> &[String] {
        &self.paths
    }

    fn probe_wdl(&self, pos: &Chess) -> Option<TbWdl> {
        #[cfg(feature = "syzygy")]
        {
            use shakmaty_syzygy::Wdl;
            let tb = self.inner.as_ref()?;
            // WDL-only probe (no DTZ required). Misses when tables absent / too many pieces.
            let wdl = tb.probe_wdl_after_zeroing(pos).ok()?;
            Some(match wdl {
                Wdl::Loss => TbWdl::Loss,
                Wdl::BlessedLoss => TbWdl::BlessedLoss,
                Wdl::Draw => TbWdl::Draw,
                Wdl::CursedWin => TbWdl::CursedWin,
                Wdl::Win => TbWdl::Win,
            })
        }

        #[cfg(not(feature = "syzygy"))]
        {
            let _ = pos;
            None
        }
    }
}

/// Default factory: prefer file-backed open; degrades to unavailable if tables absent.
pub fn open_tablebase(paths: &[String]) -> Box<dyn TablebaseProbe + Send + Sync> {
    if paths.is_empty() {
        Box::new(StubTablebase::new())
    } else {
        Box::new(FileBackedTablebase::open(paths.iter().cloned()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shakmaty::Chess;

    #[test]
    fn stub_always_misses() {
        let tb = StubTablebase::with_path("/nonexistent/syzygy");
        assert!(!tb.is_available());
        assert_eq!(tb.paths().len(), 1);
        assert!(tb.probe_wdl(&Chess::default()).is_none());
    }

    #[test]
    fn file_backed_missing_dir_is_unavailable() {
        let tb = FileBackedTablebase::open(["/this/path/does/not/exist/mossymesh-tb"]);
        assert!(!tb.is_available());
        assert!(tb.probe_wdl(&Chess::default()).is_none());
    }

    #[test]
    fn open_tablebase_empty_stub() {
        let tb = open_tablebase(&[]);
        assert!(!tb.is_available());
    }

    #[test]
    fn corpus_fingerprint_detects_divergent_tables() {
        let base = std::env::temp_dir().join("mossymesh-tb-fp-test");
        let d1 = base.join("node-a");
        let d2 = base.join("node-b");
        std::fs::create_dir_all(&d1).unwrap();
        std::fs::create_dir_all(&d2).unwrap();
        // Same file name, different sizes: divergent corpora.
        std::fs::write(d1.join("kpk.rtbw"), vec![0u8; 64]).unwrap();
        std::fs::write(d2.join("kpk.rtbw"), vec![0u8; 128]).unwrap();

        let a = FileBackedTablebase::open([d1.to_str().unwrap()]);
        let b = FileBackedTablebase::open([d2.to_str().unwrap()]);
        assert_ne!(a.corpus_fingerprint(), b.corpus_fingerprint());

        // Same corpus re-opened: stable fingerprint.
        let a2 = FileBackedTablebase::open([d1.to_str().unwrap()]);
        assert_eq!(a.corpus_fingerprint(), a2.corpus_fingerprint());

        std::fs::remove_dir_all(&base).ok();
    }
}
