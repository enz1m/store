use super::*;

impl Store {
    pub(crate) fn launch_installed(&mut self, app_id: String) -> Task<Message> {
        if self.launching_app.is_some()
            || !self.is_installed(&app_id)
            || self
                .operation_for(&app_id)
                .is_some_and(|op| !op.finished && op.error.is_none())
        {
            return Task::none();
        }
        let scope = self.installed_scope(&app_id);
        if scope == services::InstallScope::Unknown {
            return Task::none();
        }
        let Some(request) = self.allocate_owned_read_request() else {
            return Task::none();
        };
        self.launching_app = Some((app_id.clone(), request));
        let target = app_id.clone();
        let failed_app = app_id.clone();
        let work = self.read_owner.submit(request, move |context| {
            let cancel = services::OperationCancellation::default().with_blocking_context(context);
            services::launch_installed(&target, &scope, &cancel)
        });
        read_pool_task(
            work,
            move |result| Message::AppLaunched(app_id, request, result),
            move |_| {
                Message::AppLaunched(
                    failed_app,
                    request,
                    Err(
                        "Could not open this app. Refresh the installed apps and try again.".into(),
                    ),
                )
            },
        )
    }
    pub(crate) fn app_launched(
        &mut self,
        app_id: String,
        request: u64,
        result: Result<(), String>,
    ) -> Task<Message> {
        if self
            .launching_app
            .as_ref()
            .is_none_or(|(_, current)| *current != request)
        {
            return Task::none();
        }
        self.launching_app = None;
        if self.detail_app_id.as_deref() == Some(app_id.as_str()) {
            self.last_error = result.err().map(|_| {
                crate::i18n::label(
                    "Could not open this app. Refresh the installed apps and try again.",
                )
                .to_owned()
            });
        }
        Task::none()
    }
}
