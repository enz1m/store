//! Screenshot results, verification, and operations.

use super::*;

use iced::Task;

impl Store {
    pub(crate) fn apply_screenshot_result(&mut self, result: services::ScreenshotFetchResult) {
        match result {
            services::ScreenshotFetchResult::Cancelled => {}
            services::ScreenshotFetchResult::KnownEmpty => {
                self.screenshot_viewer = None;
                self.screenshots.clear();
                self.screenshot_load_state = ScreenshotLoadState::KnownEmpty;
            }
            services::ScreenshotFetchResult::Complete(shots) => {
                self.screenshot_viewer = None;
                if shots.is_empty() {
                    self.screenshots.clear();
                    self.screenshot_load_state = ScreenshotLoadState::KnownEmpty;
                } else {
                    self.screenshots = shots;
                    self.screenshot_load_state = ScreenshotLoadState::Complete;
                }
            }
            services::ScreenshotFetchResult::Partial(shots) => {
                if self.screenshot_load_state != ScreenshotLoadState::CachedStale
                    || self.screenshots.is_empty()
                {
                    self.screenshot_viewer = None;
                    self.screenshots = shots;
                    self.screenshot_load_state = ScreenshotLoadState::Partial;
                }
            }
            services::ScreenshotFetchResult::CachedStale(shots) => {
                if self.screenshot_load_state != ScreenshotLoadState::CachedStale
                    || self.screenshots.is_empty()
                {
                    self.screenshot_viewer = None;
                    self.screenshots = shots;
                    self.screenshot_load_state = ScreenshotLoadState::CachedStale;
                }
            }
            services::ScreenshotFetchResult::Failed => {
                if self.screenshot_load_state != ScreenshotLoadState::CachedStale {
                    self.screenshot_viewer = None;
                    self.screenshots.clear();
                    self.screenshot_load_state = ScreenshotLoadState::Failed;
                }
            }
        }
    }

    // Retain unavailable verification only with a known age, and explicitly mark its provenance stale.
    pub(crate) fn apply_verification(&mut self, verification: services::PublisherVerification) {
        let now = self.activity_now_ms();
        let settled_at = self.verification_settled_at;
        let Some(context) = self.detail_context.as_mut() else {
            return;
        };
        if verification.is_known() {
            context.verification = verification;
            // A disjoint field write, so it does not conflict with the
            // `detail_context` borrow held above.
            self.verification_settled_at = Some(now);
            return;
        }
        let retained = settled_at
            .map(|settled| now.saturating_sub(settled))
            .and_then(|age_ms| context.verification.retained(age_ms));
        match retained {
            Some(retained) => context.verification = retained,
            None => context.verification = verification,
        }
    }

    pub(crate) fn sync_activity_schedule(&mut self) {
        let now = self.activity_now_ms();
        let _ = self.activity_schedule.sync(&self.activity_center, now);
    }

    pub(crate) fn start_activity(
        &mut self,
        id: model::OperationId,
        key: ActivityKey,
        name: &str,
        kind: OpKind,
    ) {
        let now = self.activity_now_ms();
        let outcome = self.activity_center.publish(
            Activity::new(
                key,
                ActivityKind::Other(kind.verb()),
                format!("{} {name}", kind.verb()),
            )
            .progress(ActivityProgress::Indeterminate)
            .controls(vec![ActivityControl::Cancel]),
            now,
        );
        debug_assert_eq!(outcome, PublishOutcome::Added);
        self.active_activity_keys.insert(id, key);
        self.sync_activity_schedule();
    }

    // Raises each newly failed operation, and the close-recovery failure, as a
    // sticky error toast. The operation list keeps its "Failed" chip; a
    // recovery failure that clears withdraws its toast.
    pub(crate) fn reconcile_failure_toasts(&mut self) {
        use enzui_widgets::toast::ToastTone;
        use enzui_widgets::toast_host::ToastSpec;
        // A failure the user cleared from the list ("Dismiss failed") takes
        // its toast with it.
        let operations = &self.operations;
        let toasts = &mut self.toasts;
        self.toasted_failures
            .retain(|(app_id, attempt, error, id)| {
                let live = operations.iter().any(|op| {
                    &op.app_id == app_id
                        && op.attempt == *attempt
                        && op.error.as_ref() == Some(error)
                });
                if !live {
                    toasts.dismiss(*id);
                }
                live
            });
        for op in &self.operations {
            let Some(error) = &op.error else { continue };
            if self
                .toasted_failures
                .iter()
                .any(|(app_id, attempt, seen, _)| {
                    app_id == &op.app_id && *attempt == op.attempt && seen == error
                })
            {
                continue;
            }
            let (title, body) = crate::ui::chrome::failure_copy(
                format!("{} {} failed", op.kind.verb(), op.name),
                Some(error.clone()),
            );
            let mut spec = ToastSpec::new(title)
                .tone(ToastTone::Error)
                .coalesce_key(format!("store-operation-failure:{}", op.app_id));
            if let Some(body) = body {
                spec = spec.description(body);
            }
            let id = self.toasts.push(spec);
            self.toasted_failures
                .push((op.app_id.clone(), op.attempt, error.clone(), id));
        }
        if self.close_recovery_toast.map(|(error, _)| error) != self.close_recovery_error {
            if let Some((_, id)) = self.close_recovery_toast.take() {
                self.toasts.dismiss(id);
            }
            if let Some(error) = self.close_recovery_error {
                let (title, body) = crate::ui::chrome::failure_copy(
                    crate::i18n::label("Background results are unknown"),
                    Some(error.to_owned()),
                );
                let mut spec = ToastSpec::new(title).tone(ToastTone::Error);
                if let Some(body) = body {
                    spec = spec.description(body);
                }
                self.close_recovery_toast = Some((error, self.toasts.push(spec)));
            }
        }
    }

    fn abandon_pending_updates(&mut self) {
        use enzui_widgets::toast::ToastTone;
        use enzui_widgets::toast_host::ToastSpec;
        let skipped: Vec<String> = self
            .pending_update_reviews
            .drain(..)
            .map(|(_, name)| name)
            .collect();
        if skipped.is_empty() {
            return;
        }
        let spec = ToastSpec::new(crate::i18n::label("Remaining updates were not started"))
            .description(skipped.join(", "))
            .tone(ToastTone::Warning)
            .coalesce_key("store-update-all-skipped");
        self.toasts.push(spec);
    }

    // Finished operation history carries refusals to the normal failure surfaces.
    pub(crate) fn refuse_operation(
        &mut self,
        app_id: &str,
        name: &str,
        kind: OpKind,
        error: String,
    ) -> Task<Message> {
        let mut operation = OperationState::starting(app_id, name, kind);
        operation.message = "Refused".into();
        operation.finished = true;
        operation.error = Some(error);
        self.operations.push(operation);
        Task::none()
    }

    // Recheck mutable installation scope here; stale or synthetic UI actions must not bypass authorization.
    pub(crate) fn start_operation(
        &mut self,
        app_id: String,
        name: String,
        kind: OpKind,
    ) -> Task<Message> {
        if self
            .operations
            .iter()
            .any(|o| o.app_id == app_id && !o.finished)
        {
            return Task::none(); // already running; a second click is a no-op
        }
        if let Err(error) = flatpak_args::validate_app_id(&app_id) {
            return self.refuse_operation(&app_id, &name, kind, error);
        }
        let scope = match self.action_authorization(&app_id, kind) {
            services::Authorization::Allowed(scope) => scope,
            services::Authorization::Refused(refusal) => {
                return self.refuse_operation(&app_id, &name, kind, refusal.reason.to_owned());
            }
        };
        if self.operation_cancels.len() >= OPERATION_CONCURRENCY_LIMIT
            || self.mutation_owner.dispatcher().snapshot().live() >= OPERATION_CONCURRENCY_LIMIT
        {
            return self.refuse_operation(&app_id, &name, kind, OPERATION_BUSY.to_owned());
        }
        let reviewed_install = if matches!(kind, OpKind::Install | OpKind::Update) {
            match self.confirmed_install.take().filter(|review| {
                review.app_id == app_id && review.update == (kind == OpKind::Update)
            }) {
                Some(review) => Some(review),
                None => return self.start_transaction_consent(app_id, name, kind),
            }
        } else {
            None
        };
        let (id, activity_key) = match self.allocate_operation_ids(&app_id) {
            Ok(ids) => ids,
            Err(error) => return self.refuse_operation(&app_id, &name, kind, error.to_owned()),
        };
        self.operations
            .push(OperationState::attempt(&app_id, &name, kind, id.attempt));
        self.start_activity(id.clone(), activity_key, &name, kind);

        let cancellation = services::OperationCancellation::default();
        self.operation_cancels
            .insert(id.clone(), cancellation.clone());
        // Starting an operation never navigates: every route that offers the
        // action renders the operation in the action's own slot, and a refusal
        // reports on the page that asked for it.
        let task = flatpak::operation_task(
            &self.mutation_owner,
            flatpak::Operation {
                id,
                name,
                kind,
                scope,
                reviewed_install,
            },
            self.flatpak_program.clone(),
            cancellation,
        );
        self.operation_completions.insert(
            task.id.clone(),
            OperationCompletion {
                task: Box::pin(task.completion),
                progress_drained: task.progress_drained,
                worker_exited: task.worker_exited,
            },
        );
        task.progress.map(Message::OperationEvent)
    }

    pub(crate) fn allocate_operation_ids(
        &mut self,
        app_id: &str,
    ) -> Result<(model::OperationId, ActivityKey), &'static str> {
        let attempt = self
            .operation_attempts
            .get(app_id)
            .copied()
            .unwrap_or(0)
            .checked_add(1)
            .ok_or(OPERATION_ATTEMPT_EXHAUSTED)?;
        let next_activity_id = self
            .next_activity_id
            .checked_add(1)
            .ok_or(OPERATION_ACTIVITY_EXHAUSTED)?;
        let activity_key = ActivityKey::new(STORE_ACTIVITY_PROVIDER.0, self.next_activity_id);
        self.operation_attempts.insert(app_id.to_owned(), attempt);
        self.next_activity_id = next_activity_id;
        Ok((model::OperationId::new(app_id, attempt), activity_key))
    }

    pub(crate) fn in_flight_operation(&self, id: &model::OperationId) -> Option<&OperationState> {
        self.operations
            .iter()
            .find(|op| op.is_in_flight() && op.app_id == id.app_id && op.attempt == id.attempt)
    }

    pub(crate) fn cancel_is_actionable(&self, id: &model::OperationId) -> bool {
        self.in_flight_operation(id)
            .is_some_and(|op| op.cancel.cancel_is_actionable())
            && self.operation_cancels.contains_key(id)
    }

    pub(crate) fn poll_operation_completions(&mut self) -> Task<Message> {
        let settled = self
            .operation_completions
            .iter_mut()
            .filter_map(|(id, completion)| {
                completion
                    .poll()
                    .map(|terminal| flatpak::OpEvent::Finished {
                        id: id.clone(),
                        terminal,
                    })
            })
            .collect::<Vec<_>>();
        let tasks = settled
            .into_iter()
            .map(|event| self.apply_operation_event(event))
            .collect::<Vec<_>>();
        Task::batch(tasks)
    }

    pub(crate) fn apply_operation_event(&mut self, event: flatpak::OpEvent) -> Task<Message> {
        match event {
            flatpak::OpEvent::Progress {
                id,
                progress,
                message,
            } => {
                if model::apply_progress(&mut self.operations, &id, progress, &message)
                    && let Some(key) = self.active_activity_keys.get(&id).copied()
                    && let Some(op) = self.operations.iter().find(|op| {
                        op.is_in_flight() && op.app_id == id.app_id && op.attempt == id.attempt
                    })
                {
                    let progress = if op.sampled {
                        ActivityProgress::Fraction(op.progress)
                    } else {
                        ActivityProgress::Indeterminate
                    };
                    let _ = self
                        .activity_center
                        .progress(key, Generation::FIRST, progress);
                }
                Task::none()
            }
            flatpak::OpEvent::Finished { id, terminal } => {
                if self.in_flight_operation(&id).is_none() {
                    return Task::none();
                }
                let app_id = id.app_id.clone();
                let succeeded = matches!(&terminal, flatpak::OperationTerminal::Succeeded);
                let error = match &terminal {
                    flatpak::OperationTerminal::Succeeded => None,
                    flatpak::OperationTerminal::Failed(error) => Some(error.clone()),
                    // Stopped on request: nothing went wrong, so no failure
                    // chip and no error notification; the row reads "Cancelled".
                    flatpak::OperationTerminal::Cancelled => None,
                };
                let cancelled = matches!(&terminal, flatpak::OperationTerminal::Cancelled);
                let mut finished = None;
                if let Some(op) = self.operations.iter_mut().find(|op| {
                    op.is_in_flight() && op.app_id == id.app_id && op.attempt == id.attempt
                }) {
                    op.finished = true;
                    op.progress = 1.0;
                    op.message = match terminal {
                        flatpak::OperationTerminal::Succeeded => op.kind.past().to_owned(),
                        flatpak::OperationTerminal::Failed(_) => "Failed".to_owned(),
                        flatpak::OperationTerminal::Cancelled => "Cancelled".to_owned(),
                    };
                    op.error = error;
                    // Finished arrives only after the process group is reaped and output readers are joined.
                    if cancelled {
                        op.cancel = model::CancelState::Cancelled;
                    }
                    finished = Some((op.kind, op.name.clone()));
                }
                // Retire the handle for exactly this attempt. A retry
                // allocates a new identity, so this cannot strand or reuse a
                // token, and a stale Cancel now finds nothing to fire.
                self.operation_cancels.remove(&id);
                self.operation_completions.remove(&id);
                let activity_key = self.active_activity_keys.remove(&id);
                match (finished.as_ref(), activity_key) {
                    (Some((kind, name)), Some(key)) if succeeded => {
                        let now = self.activity_now_ms();
                        if let Some(mut row) = self.activity_center.get(key).cloned() {
                            row.label = format!("{} {name}", kind.past());
                            row.detail = None;
                            let _ = self.activity_center.publish(row, now);
                        }
                        let _ = self.activity_center.finish(
                            key,
                            Generation::FIRST,
                            TerminalOutcome::Completed,
                            now,
                        );
                        self.sync_activity_schedule();
                    }
                    (Some(_) | None, Some(key)) => {
                        let outcome = if cancelled {
                            TerminalOutcome::Cancelled
                        } else {
                            TerminalOutcome::Failed
                        };
                        let now = self.activity_now_ms();
                        let _ = self
                            .activity_center
                            .finish(key, Generation::FIRST, outcome, now);
                        // Publish the actual result before the app replaces this row with its durable alert.
                        self.sync_shell_activity();
                        let _ = self.activity_center.dismiss(key);
                        self.sync_activity_schedule();
                    }
                    (_, None) => {}
                }
                let update_finished = finished
                    .as_ref()
                    .is_some_and(|(kind, _)| *kind == OpKind::Update);
                if succeeded {
                    // The installed list is re-read below, but the action slot
                    // this operation occupied is handed back on this frame, so
                    // the local list has to agree with the outcome now or the
                    // page offers the action that just ran.
                    match finished.map(|(kind, _)| kind) {
                        Some(OpKind::Uninstall) => {
                            // Settle only the user installation; an app ID may also name a system installation.
                            let settled = self
                                .installed
                                .iter()
                                .position(|e| {
                                    e.id == app_id
                                        && e.install_scope == services::InstallScope::User
                                })
                                .or_else(|| self.installed.iter().position(|e| e.id == app_id));
                            if let Some(pos) = settled {
                                let mut entry = self.installed.remove(pos);
                                // Keep the observed source: an app no catalogue lists has no other record of it.
                                entry.removed_origin =
                                    services::RemovedOrigin::observe(&entry, &self.remotes);
                                // "Recently uninstalled" means the app is gone.
                                // If a system copy survives, it is not gone —
                                // it is still installed and still listed, and
                                // offering to reinstall it would be nonsense.
                                if !self.installed.iter().any(|e| e.id == app_id) {
                                    self.recently_uninstalled.insert(0, entry);
                                    self.recently_uninstalled.truncate(10);
                                }
                            }
                        }
                        Some(OpKind::Install) => {
                            if !self.installed.iter().any(|e| e.id == app_id)
                                && let Some(mut entry) = self.app_entry(&app_id).or_else(|| {
                                    // An app only its removal record names is otherwise absent until the re-read.
                                    self.recently_uninstalled
                                        .iter()
                                        .find(|e| e.id == app_id)
                                        .cloned()
                                })
                            {
                                entry.removed_origin = None;
                                entry.installed = true;
                                // Catalogue entries lack install scope; record the successful user installation before rereading.
                                entry.install_scope = services::InstallScope::User;
                                self.installed.push(entry);
                            }
                            self.recently_uninstalled.retain(|e| e.id != app_id);
                        }
                        _ => {}
                    }
                    // An operation settles only the installation it ran in: the user copy,
                    // or the system copy of an app installed only system-wide (the same
                    // rule update authorization and consent use).
                    let settled_scope =
                        if self.installed_scope(&app_id) == services::InstallScope::System {
                            services::InstallScope::System
                        } else {
                            services::InstallScope::User
                        };
                    self.update_discovery
                        .settle_operation(&app_id, &settled_scope);
                    self.enrich_installed();
                }
                // A completed mutation invalidates the installed list, so
                // re-read it. This is a READ, not a second mutation.
                Task::batch(vec![
                    self.start_installed_read(),
                    self.start_update_check(),
                    if update_finished {
                        if succeeded {
                            self.start_next_update_review()
                        } else {
                            self.abandon_pending_updates();
                            Task::none()
                        }
                    } else {
                        Task::none()
                    },
                ])
            }
        }
    }
}
