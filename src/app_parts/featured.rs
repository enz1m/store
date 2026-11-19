//! Featured image loading and scroll helpers.

use super::*;

use iced::Task;

impl Store {
    // Start one selected-hero load. The placeholder is `Loading` before the
    // task returns so paging has immediate feedback.
    pub(crate) fn ensure_featured_image(&mut self, app_id: String) -> Task<Message> {
        if app_id.is_empty() {
            return Task::none();
        }
        // A deferred hero holds a Loading placeholder but no live submission, so
        // paging back re-drives it; an inflight or resolved hero is reused.
        if let Some(index) = self.featured_deferred.iter().position(|id| id == &app_id) {
            self.featured_deferred.remove(index);
            return self.dispatch_featured_load(app_id);
        }
        if self.featured_images.contains_key(&app_id) {
            return Task::none();
        }
        self.dispatch_featured_load(app_id)
    }

    // Submit one hero load onto the bounded read pool under a fresh generation.
    //
    // Awaiting the returned [`BlockingTask`] offloads the metadata read, curl
    // transport and decode onto a pool worker. A saturated pool settles the
    // submission as `Busy` without running the closure.
    pub(crate) fn dispatch_featured_load(&mut self, app_id: String) -> Task<Message> {
        let Some(cache) = self.cache_cap() else {
            return Task::none();
        };
        self.featured_generation = self.featured_generation.wrapping_add(1);
        let generation = self.featured_generation;
        self.featured_generations.insert(app_id.clone(), generation);
        self.featured_images
            .insert(app_id.clone(), featured_image::HeroImageState::Loading);
        let cache_root = cache.featured_cache_path().to_path_buf();
        let refused_app = app_id.clone();
        let task = self.read_owner.submit(generation, move |_ctx| {
            featured_image::load(cache_root, app_id)
        });
        read_pool_task(
            task,
            move |outcome| Message::FeaturedImageLoaded {
                generation,
                outcome,
            },
            move |error| Message::FeaturedImageRefused {
                app_id: refused_app,
                generation,
                busy: matches!(error, enzui_core::blocking_task::BlockingTaskError::Busy),
            },
        )
    }

    // Whether a hero settlement still governs its app: it names the app's live
    // generation and the app remains a current hero page.
    pub(crate) fn featured_settlement_is_current(&self, app_id: &str, generation: u64) -> bool {
        self.featured_generations.get(app_id) == Some(&generation)
            && self.current_featured_ids().iter().any(|id| id == app_id)
    }

    // Re-drive at most one still-current retained hero intent; a retired intent
    // is dropped. A re-refused intent returns to the queue for a later tick,
    // so this never resubmits the request refused in the same turn.
    pub(crate) fn retry_one_deferred_featured(&mut self) -> Task<Message> {
        while !self.featured_deferred.is_empty() {
            let app_id = self.featured_deferred.remove(0);
            if self.current_featured_ids().iter().any(|id| id == &app_id) {
                return self.dispatch_featured_load(app_id);
            }
            self.featured_generations.remove(&app_id);
            self.featured_images.remove(&app_id);
        }
        Task::none()
    }

    // Current hero ids in fallback order. Kept effect-free so it is safe to
    // call while adopting a catalogue, before the projection cache is synced.
    pub(crate) fn current_featured_ids(&self) -> Vec<String> {
        let source = if !self.catalog.popular.is_empty() {
            &self.catalog.popular
        } else if !self.catalog.trending.is_empty() {
            &self.catalog.trending
        } else {
            &self.catalog.new
        };
        source
            .iter()
            .take(3)
            .map(|entry| entry.id.clone())
            .collect()
    }

    // Warm each bounded carousel page so paging does not reveal a fresh
    // loading placeholder.
    pub(crate) fn ensure_initial_featured_image(&mut self) -> Task<Message> {
        let tasks = self
            .current_featured_ids()
            .into_iter()
            .map(|id| self.ensure_featured_image(id));
        Task::batch(tasks)
    }

    // Send a virtual list back to the top: the realisation window this frame
    // (the geometry cell the build reads) and the scrollable's own pixel
    // offset (the operation). The old `self.*_scroll = 0.0` did only the
    // first half, and only by accident.
    pub(crate) fn scroll_geometry_to_top(
        cell: &enzui_widgets::scroll_view::ScrollGeometryCell,
        id: &'static str,
    ) -> Task<Message> {
        if let Some(g) = cell.get() {
            cell.set(enzui_widgets::scroll_view::ScrollGeometry {
                translation: iced::Vector::ZERO,
                ..g
            });
        }
        Self::scroll_to_top_operation(id)
    }

    pub(crate) fn scroll_to_top_operation(id: &'static str) -> Task<Message> {
        iced::widget::operation::scroll_to(
            iced::advanced::widget::Id::new(id),
            iced::widget::scrollable::AbsoluteOffset { x: 0.0, y: 0.0 },
            iced::advanced::widget::operation::Animation::Instant,
        )
    }
}
