//! Fourth reducer batch: catalog, install, operations, remotes.

use super::*;

use iced::Task;

impl Store {
    pub(crate) fn batch4_remotes_read_terminated(
        &mut self,
        request: u64,
        terminal: flatpak::ReadTaskError,
    ) -> Task<Message> {
        self.deferred_owned_reads.retry_settled(request);
        let intent = OwnedReadIntent::Remotes { request };
        if !self.owned_read_is_current(&intent) {
            return self.pump_deferred_owned_read();
        }
        self.remotes_load =
            services::RemoteLoad::Failed(read_terminal_text("Repository read", terminal));
        let permission_read = self.resume_detail_permission_read();
        if terminal == flatpak::ReadTaskError::Busy {
            self.retain_deferred_owned_read(intent);
            permission_read
        } else {
            Task::batch([permission_read, self.pump_deferred_owned_read()])
        }
    }

    #[allow(
        clippy::boxed_local,
        reason = "Consume the boxed event payload without copying its catalog data"
    )]
    #[cfg(test)]
    pub(crate) fn batch4_catalog_loaded(
        &mut self,
        load: Box<catalog::CatalogLoad>,
    ) -> Task<Message> {
        self.apply_catalog_load(*load)
    }

    #[allow(
        clippy::boxed_local,
        reason = "Consume the boxed event payload without copying its catalog data"
    )]
    pub(crate) fn batch4_catalog_cache_loaded(
        &mut self,
        generation: u64,
        phase: Box<CatalogCachePhase>,
    ) -> Task<Message> {
        if generation != self.catalog_generation {
            // A newer refresh advanced the generation while this cache
            // read was in flight; discarding it keeps the newer answer.
            return Task::none();
        }
        let phase = *phase;
        self.adopt_catalog(phase.paint);
        let detail = self.resume_catalog_detail();
        if phase.needs_ingest {
            Task::batch([self.dispatch_catalog_ingest(generation), detail])
        } else {
            self.loading_catalog = false;
            Task::batch([self.ensure_initial_featured_image(), detail])
        }
    }

    #[allow(
        clippy::boxed_local,
        reason = "Consume the boxed event payload without copying its catalog data"
    )]
    pub(crate) fn batch4_catalog_ingested(
        &mut self,
        generation: u64,
        load: Box<catalog::CatalogLoad>,
    ) -> Task<Message> {
        if generation != self.catalog_generation {
            // A stale boot or superseded ingest cannot overwrite the
            // catalogue a newer refresh is resolving.
            return Task::none();
        }
        self.apply_catalog_load(*load)
    }

    pub(crate) fn batch4_catalog_unavailable(
        &mut self,
        generation: u64,
        busy: bool,
    ) -> Task<Message> {
        if generation != self.catalog_generation {
            return Task::none();
        }
        self.loading_catalog = false;
        self.catalog_error = Some(if busy {
            "The catalog is busy. Try again.".to_owned()
        } else {
            "The catalog is temporarily unavailable. Try again.".to_owned()
        });
        Task::none()
    }

    pub(crate) fn batch4_install_requested(&mut self, id: String, name: String) -> Task<Message> {
        self.start_operation(id, name, OpKind::Install)
    }

    pub(crate) fn batch4_update_requested(&mut self, id: String, name: String) -> Task<Message> {
        self.start_operation(id, name, OpKind::Update)
    }

    pub(crate) fn batch4_update_all_requested(&mut self) -> Task<Message> {
        // Snapshot the pending-update set before any operation starts,
        // so a concurrent load cannot change the set mid-loop.
        let pending: Vec<_> = self
            .updates_available()
            .iter()
            // Update only apps whose update this build can apply: user-reported ones, and system
            // updates of apps installed only system-wide. Other updates remain visible.
            .filter(|update| self.update_authorization(&update.app_id).is_allowed())
            .map(|update| {
                let id = &update.app_id;
                let name = self.app_display_name(id).unwrap_or_else(|| id.clone());
                (id.clone(), name)
            })
            .collect();
        // A queue entry whose start was refused (a modal was open, a busy
        // worker, another operation on that app) leaves the queue non-empty
        // with nothing in flight. Only an actual consent dialog or running
        // update means "already updating"; otherwise rebuild the queue.
        if self.install_consent.is_some()
            || self
                .operations
                .iter()
                .any(|op| op.kind == OpKind::Update && !op.finished)
        {
            return Task::none();
        }
        self.pending_update_reviews = pending.into();
        self.start_next_update_review()
    }

    pub(crate) fn start_next_update_review(&mut self) -> Task<Message> {
        if self.install_consent.is_some()
            || self
                .operations
                .iter()
                .any(|op| op.kind == OpKind::Update && !op.finished)
        {
            return Task::none();
        }
        if let Some((id, name)) = self.pending_update_reviews.pop_front() {
            self.start_operation(id, name, OpKind::Update)
        } else {
            Task::none()
        }
    }

    pub(crate) fn batch4_uninstall_confirm_requested(
        &mut self,
        id: String,
        name: String,
    ) -> Task<Message> {
        // The guard: a removal opens a dialog, it does not remove.
        self.pending_uninstall = Some((id, name));
        self.confirm_state = enzui_core::confirm::ConfirmState::new(None);
        Task::none()
    }

    pub(crate) fn batch4_uninstall_confirm_event(
        &mut self,
        event: ConfirmDialogEvent,
    ) -> Task<Message> {
        match event {
            ConfirmDialogEvent::Resolved(cause) => {
                // `from_cause` is the single mapping from "how the dialog
                // ended" to "did the user say yes". Reading the cause
                // directly here is how a scrim press becomes consent.
                let confirmed =
                    enzui_core::confirm::ConfirmDecision::from_cause(cause).is_confirmed();
                let pending = self.pending_uninstall.take();
                match (confirmed, pending) {
                    (true, Some((id, name))) => self.start_operation(id, name, OpKind::Uninstall),
                    _ => Task::none(),
                }
            }

            ConfirmDialogEvent::FocusNext | ConfirmDialogEvent::FocusPrevious => {
                // The confirm dialog is the only presentation, so it is
                // always topmost when open.
                match event.focus_operation::<Message>(true) {
                    Some(op) => iced::advanced::widget::operate(op),
                    None => Task::none(),
                }
            }
            ConfirmDialogEvent::CheckboxToggled(_) => Task::none(),
        }
    }

    pub(crate) fn batch4_focus_next(&mut self) -> Task<Message> {
        // Modal hosts own Tab; a second outer traversal would move focus outside the scrim.
        if self.a_modal_is_open() {
            Task::none()
        } else {
            operate(traverse::<Message>(Traversal::Next))
        }
    }

    // Ctrl+F opens the search popup (or keeps it open) with the caret in
    // its field; on the Search page, the page's own field takes it.
    pub(crate) fn batch4_focus_search(&mut self) -> Task<Message> {
        if self.a_modal_is_open() {
            return Task::none();
        }
        if self.route == Route::Search {
            return operate(iced::advanced::widget::operation::focusable::focus(
                iced::advanced::widget::Id::new(crate::ui::search::INLINE_SEARCH_FIELD_ID),
            ));
        }
        self.open_search_popup()
    }

    // Open the search popup on the current query, focused on its field.
    pub(crate) fn open_search_popup(&mut self) -> Task<Message> {
        self.search_popup_open = true;
        let icons = self.refresh_search_popup();
        let focus = operate(iced::advanced::widget::operation::focusable::focus(
            iced::advanced::widget::Id::new(crate::ui::search_popup::SEARCH_POPUP_FIELD_ID),
        ));
        Task::batch([icons, focus])
    }

    // The popup's best matches for the query, computed once per edit rather
    // than on every frame; any of them still without an icon has it fetched.
    pub(crate) fn refresh_search_popup(&mut self) -> Task<Message> {
        let rows = crate::model::search_rows_from(
            self.catalog_candidates().into_iter(),
            &self.search_query,
            self.filters,
            &self.installed_ids(),
            &self.update_ids(),
        );
        self.search_popup_total = rows.len();
        self.search_popup_results = rows
            .into_iter()
            .take(crate::ui::search_popup::SEARCH_POPUP_RESULTS)
            .collect();
        let missing: Vec<services::AppEntry> = self.search_popup_results.clone();
        self.fetch_row_icons(missing)
    }

    // Fetch, off the UI thread, the icons of `entries` that have none and
    // were not asked for already; they arrive as `RowIconsLoaded`.
    pub(crate) fn fetch_row_icons(&mut self, entries: Vec<services::AppEntry>) -> Task<Message> {
        let missing: Vec<services::AppEntry> = entries
            .into_iter()
            .filter(|entry| entry.icon_image.is_none() && !entry.icon_url.is_empty())
            .filter(|entry| self.icons_requested.insert(entry.id.clone()))
            .collect();
        if missing.is_empty() {
            return Task::none();
        }
        let Some(cache) = self.cache_cap() else {
            return Task::none();
        };
        // The generation only labels the settlement; icons are applied
        // whenever they arrive, since an icon is right for its app at any time.
        let task = self.read_owner.submit(0, move |_ctx| {
            services::cache_icons_for(cache.icons(), &missing, &|| false)
                .into_iter()
                .collect::<Vec<_>>()
        });
        read_pool_task(task, Message::RowIconsLoaded, |_| {
            Message::RowIconsLoaded(Vec::new())
        })
    }

    // Icons fetched for listed rows: onto the popup's matches, and into the
    // catalogue so every list showing those apps has them.
    pub(crate) fn apply_row_icons(
        &mut self,
        icons: Vec<(String, crate::bounded_image::RemoteImage)>,
    ) {
        if icons.is_empty() {
            return;
        }
        let icons: std::collections::HashMap<_, _> = icons.into_iter().collect();
        for entry in &mut self.search_popup_results {
            if entry.icon_image.is_none()
                && let Some(icon) = icons.get(&entry.id)
            {
                entry.icon_image = Some(crate::bounded_image::RenderImage::Remote(icon.clone()));
            }
        }
        self.catalog_index.apply_icons(&icons);
    }

    // A self-realising list scrolled: record the offset its next build
    // realises rows at, and fetch icons for the rows now in view.
    pub(crate) fn list_scrolled(
        &mut self,
        geometry: &enzui_widgets::scroll_view::ScrollGeometryCell,
        offset: f32,
    ) -> Task<Message> {
        // The widget usually published this offset already; set it anyway
        // for a report that arrives first. Asking for icons is cheap when
        // they are in hand or requested: `fetch_row_icons` skips those.
        if let Some(mut measured) = geometry.get()
            && measured.translation.y != offset
        {
            measured.translation.y = offset;
            geometry.set(measured);
        }
        self.fetch_visible_row_icons(geometry)
    }

    // Icons for the rows of the current list that a viewport at the
    // geometry's offset shows, with a screen of margin below.
    pub(crate) fn fetch_visible_row_icons(
        &mut self,
        geometry: &enzui_widgets::scroll_view::ScrollGeometryCell,
    ) -> Task<Message> {
        // Rows read mid-update must reflect this message's changes.
        self.sync_projection();
        let (offset, height) = geometry.get().map_or((0.0, 1000.0), |g| {
            (g.translation.y, g.bounds.height.max(1.0))
        });
        let pitch = crate::ui::chrome::row_pitch().max(1.0);
        let first = (offset / pitch).floor().max(0.0) as usize;
        let count = ((2.0 * height) / pitch).ceil() as usize + 1;
        let visible: Vec<services::AppEntry> = self
            .rows()
            .iter()
            .skip(first)
            .take(count)
            .cloned()
            .collect();
        self.fetch_row_icons(visible)
    }

    // Every result for the query, on the Search page with its filters; the
    // query joins the recent searches as Enter in a search field does.
    pub(crate) fn search_popup_show_all(&mut self) -> Task<Message> {
        self.search_popup_open = false;
        if self.search_query.trim().is_empty() {
            return Task::none();
        }
        let saved = self.batch2_search_submitted();
        if self.route != Route::Search {
            self.previous_route = self.route;
            self.route = Route::Search;
        }
        let to_top = Self::scroll_geometry_to_top(&self.search_scroll, SEARCH_LIST_ID);
        let geometry = self.search_scroll.clone();
        let icons = self.fetch_visible_row_icons(&geometry);
        Task::batch([saved, to_top, icons])
    }

    pub(crate) fn batch4_focus_previous(&mut self) -> Task<Message> {
        if self.a_modal_is_open() {
            Task::none()
        } else {
            operate(traverse::<Message>(Traversal::Previous))
        }
    }

    pub(crate) fn batch4_operation_event(&mut self, event: flatpak::OpEvent) -> Task<Message> {
        self.apply_operation_event(event)
    }

    pub(crate) fn batch4_operation_poll(&mut self) -> Task<Message> {
        self.poll_operation_completions()
    }

    pub(crate) fn batch4_cancel_operation_requested(
        &mut self,
        id: model::OperationId,
    ) -> Task<Message> {
        if !self.cancel_is_actionable(&id) {
            return Task::none();
        }
        // Signal first, then record. The token is what stops the
        // child; the row state is only what the page says about it.
        if let Some(cancellation) = self.operation_cancels.get(&id) {
            cancellation.cancel();
        }
        if let Some(op) = self
            .operations
            .iter_mut()
            .find(|op| op.is_in_flight() && op.app_id == id.app_id && op.attempt == id.attempt)
        {
            // NOT terminal. The group has been signalled, not reaped:
            // the worker still has to wait out `TERM_GRACE`, escalate
            // to KILL, reap the leader and join both readers. The row
            // says "Cancelling…" until `OpEvent::Finished` arrives,
            // because claiming success while a child survives is the
            // one thing this issue forbids outright.
            op.cancel = model::CancelState::Cancelling;
            op.message = CANCELLING_MESSAGE.to_owned();
        }
        // The Activity row loses its Cancel control for the same
        // reason the button goes disabled: it is no longer actionable,
        // and it must not vanish.
        if let Some(key) = self.active_activity_keys.get(&id).copied()
            && let Some(mut row) = self.activity_center.get(key).cloned()
        {
            row.controls.clear();
            row.detail = Some(CANCELLING_MESSAGE.to_owned());
            let now = self.activity_now_ms();
            let _ = self.activity_center.publish(row, now);
            self.sync_activity_schedule();
        }
        Task::none()
    }

    pub(crate) fn batch4_activity_prune(&mut self, deadline: u64) -> Task<Message> {
        self.activity_schedule.fired();
        let _ = self.activity_center.prune(deadline);
        self.sync_activity_schedule();
        Task::none()
    }

    pub(crate) fn batch4_clear_finished_tasks(&mut self) -> Task<Message> {
        self.operations.retain(|o| !o.finished);
        let terminal: Vec<_> = self
            .activity_center
            .iter()
            .filter(|row| row.phase.is_terminal())
            .map(|row| row.key)
            .collect();
        for key in terminal {
            let _ = self.activity_center.dismiss(key);
        }
        self.sync_activity_schedule();
        Task::none()
    }

    pub(crate) fn batch4_clear_recently_uninstalled(&mut self) -> Task<Message> {
        self.recently_uninstalled.clear();
        Task::none()
    }

    pub(crate) fn batch4_add_remote_opened(&mut self) -> Task<Message> {
        self.open_add_remote()
    }

    pub(crate) fn batch4_add_remote_name_changed(&mut self, name: String) -> Task<Message> {
        self.edit_add_remote_draft(|form| form.name = name);
        Task::none()
    }

    pub(crate) fn batch4_add_remote_url_changed(&mut self, url: String) -> Task<Message> {
        self.edit_add_remote_draft(|form| form.url = url);
        Task::none()
    }

    pub(crate) fn batch4_add_remote_submit(&mut self) -> Task<Message> {
        let Some(form) = self.add_remote_form.as_ref() else {
            return Task::none();
        };
        let name = form.name.trim().to_owned();
        let url = form.url.trim().to_owned();
        // Validate BEFORE shelling out. A bad URL is never handed to
        // `flatpak remote-add`; the error is shown inline instead.
        match model::validate_remote(&name, &url) {
            Ok(()) => {
                // Read current remote configuration before mutation; keep the draft until conflicts and failures settle.
                if let Some(form) = self.add_remote_form.as_mut() {
                    form.error = None;
                    form.conflict = None;
                    form.busy = true;
                }
                let generation = self.add_remote_generation;
                let task = self.read_owner.submit(generation, move |_ctx| {
                    services::plan_remote_add_request(&name, &url)
                });
                read_pool_task(
                    task,
                    move |result| Message::RemoteAddPlanned(generation, result),
                    move |_error| {
                        Message::RemoteAddPlanned(
                            generation,
                            Err(READ_POOL_REMOTE_REFUSAL.to_owned()),
                        )
                    },
                )
            }
            Err(err) => {
                if let Some(form) = self.add_remote_form.as_mut() {
                    form.error = Some(err.message().to_owned());
                }
                Task::none()
            }
        }
    }

    pub(crate) fn batch4_add_remote_cancelled(&mut self) -> Task<Message> {
        self.dismiss_add_remote(enzui_core::modal::DismissCause::CancelAction)
    }
}
