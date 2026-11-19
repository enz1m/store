//! Row-projection Store accessors.

use super::*;

impl Store {
    // Reuse the row projection until a state input changes.
    pub(crate) fn projection(&self) -> &projection::RowProjection {
        // A filled cell must always be current: `update` clears it whenever a
        // key input moves. The only way to hold a stale one is to mutate a
        // `Store` field directly *after* a build, which no production path
        // does and which a test must not do silently — a cache serving stale
        // rows is the failure mode this whole change has to avoid. The check
        // is `cfg(test)` rather than `debug_assert` so the running app never
        // pays for the key hash on a layout pass.
        #[cfg(test)]
        if projection::staleness_checks_enabled()
            && let Some(p) = self.projection.get()
        {
            assert!(
                p.matches(&projection::ProjectionKey::of(self)),
                "stale row projection: a Store field that feeds the projection was \
             mutated directly after a view build. Drive the change through \
             Store::update(..), or call Store::refresh_projection() after \
             poking fields in a test."
            );
        }
        self.projection.get_or_init(|| {
            self.projection_builds.set(self.projection_builds.get() + 1);
            projection::RowProjection::compute(self)
        })
    }

    // The rows the current route shows.
    pub(crate) fn rows(&self) -> &[AppEntry] {
        self.projection().rows()
    }

    pub(crate) fn popular_rows(&self) -> &[AppEntry] {
        self.projection().popular()
    }

    // Drop the projection when (and only when) one of its inputs moved.
    //
    // Called from [`Store::update`] after every message, so an unrelated
    // message costs one key comparison and no projection work at all.
    // Force the projection to be recomputed on the next read.
    //
    // Tests that assemble a `Store` by assigning fields directly and then
    // render more than once need this; the app itself never does, because
    // every mutation goes through [`Store::update`].
    #[cfg(test)]
    pub(crate) fn refresh_projection(&mut self) {
        self.projection.take();
    }

    /// What the Library's rows show of running work: each operation's app,
    /// attempt, stage and whole-percent progress.
    fn operations_signature(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        for op in &self.operations {
            op.app_id.hash(&mut h);
            op.attempt.hash(&mut h);
            op.finished.hash(&mut h);
            op.error.is_some().hash(&mut h);
            op.sampled.hash(&mut h);
            ((op.progress * 100.0).round() as u32).hash(&mut h);
            op.cancel.hash(&mut h);
        }
        h.finish()
    }

    pub(crate) fn sync_projection(&mut self) {
        let stale = match self.projection.get() {
            Some(p) => !p.matches(&projection::ProjectionKey::of(self)),
            None => false,
        };
        if stale {
            self.projection.take();

            self.library_cache.invalidate();
        }
        // The Library's rows show running work, so a change in it rebuilds them.
        let operations = self.operations_signature();
        if operations != self.library_operations_signature {
            self.library_operations_signature = operations;
            self.library_cache.invalidate();
        }
    }

    /// Borrow the routed row provider; externally supplied rows are retained in Store-owned storage.
    pub(crate) fn row_provider<'a>(
        &'a self,
        rows: &'a [AppEntry],
    ) -> &'a dyn enzui_core::virtual_sequence::KeyedProvider<AppEntry> {
        let projected = self.projection();
        if std::ptr::eq(projected.rows(), rows) || projected.rows() == rows {
            return projected;
        }
        let adopted = self
            .adopted_rows
            .get_or_init(|| projection::RowProjection::adopted(rows.to_vec()));
        debug_assert_eq!(
            adopted.rows(),
            rows,
            "row_provider adopted a second, different foreign row slice; \
         a direct-view caller must use one Store per row set"
        );
        adopted
    }
}
