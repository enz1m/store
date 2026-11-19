//! Detail generations and close lifecycle.

use super::*;

use iced::Task;

impl Store {
    // Both app ID and visit generation must match; returning to the same app creates a new visit.
    pub(crate) fn detail_read_is_current(&self, app_id: &str, generation: u64) -> bool {
        self.route == Route::Detail
            && self.detail_app_id.as_deref() == Some(app_id)
            && generation == self.detail_generation
    }

    pub(crate) fn next_detail_generation(&mut self) -> Option<u64> {
        self.detail_cancellation.cancel();
        let Some(generation) = self.detail_generation.checked_add(1) else {
            self.refuse_detail_generation();
            return None;
        };
        if generation == 0
            || self
                .cache_cap()
                .is_some_and(|cache| !cache.admit_detail_generation(generation))
        {
            self.refuse_detail_generation();
            return None;
        }
        self.detail_generation = generation;
        self.detail_cancellation = services::OperationCancellation::default();
        self.detail_generation_exhausted = false;
        self.detail_request_error = None;
        Some(generation)
    }

    pub(crate) fn cancel_detail_requests(&mut self) {
        self.detail_cancellation.cancel();
        if self.detail_generation_exhausted {
            return;
        }
        let Some(generation) = self.detail_generation.checked_add(1) else {
            self.refuse_detail_generation();
            return;
        };
        if generation == 0
            || self
                .cache_cap()
                .is_some_and(|cache| !cache.admit_detail_generation(generation))
        {
            self.refuse_detail_generation();
            return;
        }
        self.detail_generation = generation;
        self.detail_cancellation = services::OperationCancellation::default();
    }

    pub(crate) fn refuse_detail_generation(&mut self) {
        self.detail_cancellation.cancel();
        self.detail_generation_exhausted = true;
        self.detail_request_error = Some(DETAIL_GENERATION_EXHAUSTED);
        if let Some(cache) = self.cache_cap() {
            cache.close_detail_publication();
        }
    }

    pub(crate) fn begin_close(
        &mut self,
        window: iced::window::Id,
        reason: enzui_core::portal::CancelReason,
    ) -> Task<Message> {
        if self.closing_window.is_some() {
            return Task::none();
        }
        self.closing = true;
        self.closing_window = Some(window);
        self.close_drain_session = None;
        self.close_retained_owners = None;
        self.close_drain_report = None;
        self.close_helper_status = CloseHelperStatus::NotStarted;
        self.close_started = Some(std::time::Instant::now());
        self.close_emitted = false;
        self.close_keep_open_requested = false;
        self.detail_cancellation.cancel();
        self.installed_read_request = 0;
        self.remotes_read_request = 0;
        self.updates_read_request = 0;
        self.permission_toggles_read_request = 0;
        self.deferred_owned_reads = DeferredOwnedReads::default();
        self.close_operation_ids = self.operation_cancels.keys().cloned().collect();
        for id in &self.close_operation_ids {
            if let Some(cancellation) = self.operation_cancels.get(id) {
                cancellation.cancel();
            }
        }
        if let Some(cache) = self.cache_cap() {
            cache.close_detail_publication();
        }
        self.url_launcher.cancel_and_settle(reason);

        let queue = std::num::NonZeroUsize::new(1).expect("close placeholder queue is nonzero");
        let owners = CloseOwners {
            catalog: std::mem::replace(
                &mut self.catalog_owner,
                enzui_core::blocking_task::BlockingTaskOwner::unavailable(queue),
            ),
            detail: std::mem::replace(
                &mut self.detail_owner,
                enzui_core::blocking_task::BlockingTaskOwner::unavailable(queue),
            ),
            reads: std::mem::replace(
                &mut self.read_owner,
                enzui_core::blocking_task::BlockingTaskOwner::unavailable(queue),
            ),
            mutations: std::mem::replace(
                &mut self.mutation_owner,
                enzui_core::blocking_task::BlockingTaskOwner::unavailable(queue),
            ),
            exports: std::mem::replace(&mut self.export_owner, StoreExportOwner::unavailable()),
        };
        self.close_drain_probe = Some(owners.probe());
        match start_close_drain(owners) {
            Ok(mut session) => {
                let (report, helper) = session.poll(false, false);
                self.close_drain_report = Some(report);
                self.close_helper_status = helper;
                self.close_drain_session = Some(session);
            }
            Err(owners) => {
                self.close_drain_report = Some(close_drain_start_failed());
                self.close_retained_owners = Some(owners);
            }
        }
        Task::done(Message::CloseDrainPoll)
    }

    pub(crate) fn retry_close_drain(&mut self) -> Task<Message> {
        self.close_keep_open_requested = false;
        if self
            .close_drain_report
            .is_some_and(CloseDrainReport::start_failed)
            && let Some(owners) = self.close_retained_owners.take()
        {
            self.close_started = Some(std::time::Instant::now());
            self.close_drain_probe = Some(owners.probe());
            match start_close_drain(owners) {
                Ok(mut session) => {
                    let (report, helper) = session.poll(false, false);
                    self.close_drain_report = Some(report);
                    self.close_helper_status = helper;
                    self.close_drain_session = Some(session);
                }
                Err(owners) => {
                    self.close_drain_report = Some(close_drain_start_failed());
                    self.close_retained_owners = Some(owners);
                }
            }
        }
        Task::done(Message::CloseDrainPoll)
    }

    pub(crate) fn poll_close_drain(&mut self) -> Task<Message> {
        let Some(window) = self.closing_window else {
            return Task::none();
        };
        if let Some(session) = self.close_drain_session.as_mut() {
            let elapsed = self.close_started.map(|started| started.elapsed());
            let deadline_expired = elapsed.is_some_and(|elapsed| elapsed >= CLOSE_OWNER_WAIT);
            let helper_overtime =
                elapsed.is_some_and(|elapsed| elapsed >= CLOSE_HELPER_CUSTODY_LIMIT);
            let (report, helper) = session.poll(deadline_expired, helper_overtime);
            self.close_drain_report = Some(report);
            self.close_helper_status = helper;
        }
        let Some(report) = self.close_drain_report else {
            return Task::none();
        };
        if self
            .close_operation_ids
            .iter()
            .any(|id| self.operation_cancels.contains_key(id))
        {
            return Task::none();
        }
        if close_is_authorized(report, self.close_helper_status) {
            if self.close_keep_open_requested {
                self.restore_after_close(None, false);
                return Task::none();
            }
            if !self.close_emitted {
                self.close_emitted = true;
                #[cfg(feature = "center")]
                self.shutdown_center();
                return iced::window::close(window);
            }
            return Task::none();
        }
        if !self.close_keep_open_requested {
            return Task::none();
        }
        let all_old_work_exited = self
            .close_drain_probe
            .as_ref()
            .is_some_and(|probe| probe.live() == 0);
        if !all_old_work_exited {
            return Task::none();
        }
        if report.start_failed() {
            if let Some(owners) = self.close_retained_owners.take() {
                self.restore_after_close(Some(owners), false);
            }
        } else if !report.still_joining()
            && matches!(
                self.close_helper_status,
                CloseHelperStatus::Joined | CloseHelperStatus::Panicked
            )
        {
            self.restore_after_close(
                None,
                report.has_panicked() || self.close_helper_status == CloseHelperStatus::Panicked,
            );
        }
        Task::none()
    }

    pub(crate) fn restore_after_close(&mut self, retained: Option<CloseOwners>, uncertain: bool) {
        if let Some(owners) = retained {
            self.catalog_owner = owners.catalog;
            self.detail_owner = owners.detail;
            self.read_owner = owners.reads;
            self.mutation_owner = owners.mutations;
            self.export_owner = owners.exports;
        } else {
            self.catalog_owner = blocking_owner(
                "store-catalog",
                CATALOG_WORKER_COUNT,
                CATALOG_QUEUE_CAPACITY,
            );
            self.detail_owner =
                blocking_owner("store-detail", DETAIL_WORKER_COUNT, DETAIL_QUEUE_CAPACITY);
            self.read_owner = blocking_owner("store-read", READ_WORKER_COUNT, READ_QUEUE_CAPACITY);
            self.mutation_owner = blocking_owner(
                "store-mutation",
                OPERATION_CONCURRENCY_LIMIT,
                OPERATION_QUEUE_CAPACITY,
            );
            self.export_owner =
                StoreExportOwner::start().unwrap_or_else(|_| StoreExportOwner::unavailable());
        }

        self.closing = false;
        self.closing_window = None;
        self.close_drain_session = None;
        self.close_retained_owners = None;
        self.close_drain_probe = None;
        self.close_drain_report = None;
        self.close_operation_ids.clear();
        self.close_helper_status = CloseHelperStatus::NotStarted;
        self.close_started = None;
        self.close_emitted = false;
        self.close_keep_open_requested = false;

        let recovery = if uncertain {
            CLOSE_RECOVERY_PANICKED
        } else {
            CLOSE_RECOVERY_UNKNOWN
        };
        self.close_recovery_error = Some(recovery);
        self.detail_request_error = Some(recovery);
        self.detail_permissions = PermissionRead::Unavailable;
        self.permission_toggles = PermissionTogglesState::Unavailable;
        self.detail_context = Some(services::DetailContext::failed());
        self.screenshots.clear();
        self.screenshot_load_state = ScreenshotLoadState::Failed;
        self.loading_catalog = false;
        self.loading_installed = false;
        self.remotes_load = services::RemoteLoad::Failed(recovery.to_owned());

        let Some(generation) = self.detail_generation.checked_add(1) else {
            self.refuse_detail_generation();
            return;
        };
        let cache_resumed = self
            .cache_cap()
            .is_none_or(|cache| cache.resume_detail_publication(generation));
        if cache_resumed && generation != 0 {
            self.detail_generation = generation;
            self.detail_cancellation = services::OperationCancellation::default();
            self.detail_generation_exhausted = false;
        } else {
            self.refuse_detail_generation();
        }
    }
}
