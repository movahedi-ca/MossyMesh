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

/// File-backed tablebase handle.
///
/// - Without `syzygy` feature: behaves as a path-aware stub (no I/O, always miss).
/// - With `syzygy`: loads via `shakmaty_syzygy::Tablebase` when directories exist.
/// - With `syzygy-mmap`: prefers mmap filesystem when the dependency enables it.
pub struct FileBackedTablebase {
    paths: Vec<String>,
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
            #[cfg(feature = "syzygy")]
            inner: None,
        }
    }

    /// Open tablebase directories. If none can be opened (missing files), remains a stub-like miss.
    pub fn open(paths: impl IntoIterator<Item = impl Into<String>>) -> Self {
        let paths: Vec<String> = paths.into_iter().map(Into::into).collect();

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
                inner: if any { Some(tb) } else { None },
            };
        }

        #[cfg(not(feature = "syzygy"))]
        {
            let _ = &paths;
            Self { paths }
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
}
