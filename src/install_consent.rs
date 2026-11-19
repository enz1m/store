//! Resolved-install permission consent with owned work and modal focus.
use crate::{Message, Store, flatpak, model::OpKind, services};
use enzui_core::modal::DismissCause;
use enzui_widgets::modal_host::{ModalFocusLifecycle, ModalHostEvent};
use iced::{Element, Length, Task};
use serde_json::{Value, json};

const SCOPE: &str = "store-install-consent";

#[derive(Clone, Debug)]
pub(crate) struct Restriction {
    pub label: String,
    pub category: String,
    pub value: String,
    pub blocked: bool,
}

pub(crate) struct Consent {
    pub generation: u64,
    pub kind: OpKind,
    pub app_id: String,
    pub name: String,
    pub remote: String,
    /// URL a removal record bound this review to; the resolved plan must agree.
    pub source_url: Option<String>,
    /// Branch and arch a removal record names; the review and the install use both.
    pub target: services::install_review::RefTarget,
    pub cancellation: services::OperationCancellation,
    pub focus: ModalFocusLifecycle,
    pub plan: Option<Value>,
    pub error: Option<String>,
    pub restrictions: Vec<Restriction>,
    pub details: bool,
    /// Updating an app that is installed only system-wide.
    pub system: bool,
}

impl Store {
    #[cfg(test)]
    pub(crate) fn start_install_consent(&mut self, app_id: String, name: String) -> Task<Message> {
        self.start_transaction_consent(app_id, name, OpKind::Install)
    }

    pub(crate) fn start_transaction_consent(
        &mut self,
        app_id: String,
        name: String,
        kind: OpKind,
    ) -> Task<Message> {
        if self.install_consent.is_some() || self.a_modal_is_open() {
            return Task::none();
        }
        // An update of an app installed only system-wide (preinstalled Zen) goes to the
        // system installation and its system remote.
        let system = kind == OpKind::Update
            && self.installed_scope(&app_id) == services::InstallScope::System;
        let wanted = if system {
            services::InstallScope::System
        } else {
            services::InstallScope::User
        };
        let origin = self
            .installed
            .iter()
            .find(|entry| entry.id == app_id && entry.install_scope == wanted)
            .map(|entry| entry.origin.clone())
            .or_else(|| self.app_entry(&app_id).map(|entry| entry.origin));
        // Same lookup order as install_authorization: a removal record speaks only for apps nothing else names.
        let removed = if origin.is_none() {
            self.removed_origin(&app_id).cloned()
        } else {
            None
        };
        let remote = if let Some(removed) = &removed {
            removed
                .current_remote(&self.remotes)
                .filter(|_| removed.scope == services::InstallScope::User)
                .map(|remote| remote.name.clone())
        } else {
            let origin = origin.unwrap_or_default();
            let known_url = services::KNOWN_CATALOG_REPOSITORIES
                .iter()
                .find(|repo| repo.origin == origin)
                .map(|repo| services::normalise_repository_url(repo.url));
            self.remotes
                .iter()
                .find(|remote| {
                    services::InstallScope::from_remote_options(&remote.options) == wanted
                        && (remote.name == origin
                            || known_url.as_deref().is_some_and(|url| {
                                services::normalise_repository_url(&remote.url) == url
                            }))
                })
                .map(|remote| remote.name.clone())
        };
        let Some(remote) = remote else {
            let which = if system { "system" } else { "user" };
            return self.refuse_operation(
                &app_id,
                &name,
                kind,
                crate::i18n::format(
                    crate::i18n::localizer(),
                    "No eligible {which} repository is available",
                    &[("which", &(which))],
                ),
            );
        };
        let target = removed
            .as_ref()
            .map(|removed| {
                services::install_review::RefTarget::recorded(&removed.branch, &removed.arch)
            })
            .unwrap_or_default();
        // Without the permission service (any desktop but enzimOS) there is no
        // review to wait for: Flatpak installs or updates the app directly.
        if !self.permission_service {
            self.confirmed_install = Some(services::install_review::ReviewedInstall {
                app_id: app_id.clone(),
                update: kind == OpKind::Update,
                remote,
                target,
                token: String::new(),
                changes: Vec::new(),
                system,
                direct: true,
            });
            return self.start_operation(app_id, name, kind);
        }
        let Some(generation) = self.install_review_generation.checked_add(1) else {
            return Task::none();
        };
        self.install_review_generation = generation;
        let cancellation = services::OperationCancellation::default();
        let (focus, open) = ModalFocusLifecycle::open::<Message>(
            SCOPE,
            (self.route == crate::Route::Detail).then(|| {
                iced::advanced::widget::Id::from(format!(
                    "store-detail-{}:{app_id}",
                    if kind == OpKind::Update {
                        "update"
                    } else {
                        "install"
                    }
                ))
            }),
        );
        self.install_consent = Some(Consent {
            generation,
            kind,
            app_id: app_id.clone(),
            name,
            remote: remote.clone(),
            source_url: removed.map(|removed| removed.url),
            target: target.clone(),
            cancellation: cancellation.clone(),
            focus,
            plan: None,
            error: None,
            restrictions: Vec::new(),
            details: false,
            system,
        });
        let read = flatpak::owned_read_task(
            self.detail_owner.dispatcher(),
            generation,
            cancellation,
            move |token| {
                services::install_review::prepare(
                    &app_id,
                    &remote,
                    &target,
                    kind == OpKind::Update,
                    system,
                    &token,
                )
            },
            move |result| Message::InstallReviewLoaded(generation, result),
            move |terminal| {
                Message::InstallReviewLoaded(
                    generation,
                    Err(crate::i18n::format(
                        crate::i18n::localizer(),
                        "Permission review unavailable: {}",
                        &[("arg0", &(terminal.message()))],
                    )),
                )
            },
        );
        Task::batch([iced::advanced::widget::operate(open), read])
    }

    pub(crate) fn install_review_loaded(
        &mut self,
        generation: u64,
        result: Result<Value, String>,
    ) -> Task<Message> {
        let Some(consent) = self
            .install_consent
            .as_mut()
            .filter(|state| state.generation == generation)
        else {
            return Task::none();
        };
        match result {
            Ok(plan) => {
                if let Err(error) =
                    services::install_review::validate_plan(&plan, &consent.app_id, &consent.remote)
                {
                    consent.error = Some(error);
                    return Task::none();
                }
                if (plan["action"] == "update") != (consent.kind == OpKind::Update) {
                    consent.error =
                        Some(crate::i18n::label("Permission review action changed").into());
                    return Task::none();
                }
                if let (Some(expected), Some(url)) =
                    (&consent.source_url, plan["remote"]["url"].as_str())
                    && services::normalise_repository_url(expected)
                        != services::normalise_repository_url(url)
                {
                    consent.error = Some(
                        crate::i18n::label(
                            "The repository's address changed since this app was removed",
                        )
                        .into(),
                    );
                    return Task::none();
                }
                if !resolves_target(&plan, &consent.app_id, &consent.target) {
                    consent.error = Some(crate::i18n::label("The repository resolved a different version of this app than the one removed").into());
                    return Task::none();
                }
                consent.restrictions = restrictions(&plan, &consent.app_id);
                consent.plan = Some(plan);
                // Install and Update act at once with the app's requested access; people
                // restrict it later from the app's permissions. The review still pins the
                // exact app and version, and a failed review is shown instead of installing.
                return self.confirm_install_consent();
            }
            Err(error) => consent.error = Some(error),
        }
        Task::none()
    }

    pub(crate) fn cancel_install_consent(&mut self) -> Task<Message> {
        self.confirmed_install = None;
        self.pending_update_reviews.clear();
        self.install_consent
            .take()
            .map_or_else(Task::none, |consent| {
                consent.cancellation.cancel();
                consent
                    .focus
                    .reduce::<Message>(ModalHostEvent::Dismissed(DismissCause::CancelAction))
                    .into_task()
            })
    }

    pub(crate) fn install_consent_host(&mut self, event: ModalHostEvent) -> Task<Message> {
        let Some(consent) = self.install_consent.as_ref() else {
            return Task::none();
        };
        let outcome = consent.focus.reduce::<Message>(event);
        if outcome.dismissed().is_some() {
            if let Some(consent) = self.install_consent.take() {
                consent.cancellation.cancel();
            }
            self.confirmed_install = None;
            self.pending_update_reviews.clear();
        }
        outcome.into_task()
    }

    pub(crate) fn block_install_permission(
        &mut self,
        index: usize,
        blocked: bool,
    ) -> Task<Message> {
        if let Some(consent) = self.install_consent.as_mut()
            && let Some(restriction) = consent.restrictions.get_mut(index)
        {
            restriction.blocked = blocked;
        }
        Task::none()
    }

    pub(crate) fn confirm_install_consent(&mut self) -> Task<Message> {
        let Some(consent) = self.install_consent.as_ref() else {
            return Task::none();
        };
        if consent.error.is_some() || consent.plan.is_none() {
            return Task::none();
        }
        let consent = self.install_consent.take().expect("checked consent");
        let plan = consent.plan.expect("checked resolved plan");
        let close = consent.focus.close_for_host::<Message>().into_task();
        self.confirmed_install = Some(services::install_review::ReviewedInstall {
            app_id: consent.app_id.clone(), update: consent.kind == OpKind::Update, remote: consent.remote, target: consent.target, token: plan["review_token"].as_str().expect("validated token").into(),
            changes: consent.restrictions.into_iter().filter(|restriction| restriction.blocked)
                .map(|restriction| json!({"category":restriction.category,"value":restriction.value,"allow":false})).collect(),
            system: consent.system,
            direct: false,
        });
        Task::batch([
            close,
            self.start_operation(consent.app_id, consent.name, consent.kind),
        ])
    }
}

/// A pinned reinstall's review must resolve the app on the recorded branch and arch.
fn resolves_target(
    plan: &Value,
    app_id: &str,
    target: &services::install_review::RefTarget,
) -> bool {
    if target.branch.is_none() && target.arch.is_none() {
        return true;
    }
    plan["operations"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|op| op["ref"].as_str())
        .any(|reference| {
            let parts = reference.split('/').collect::<Vec<_>>();
            parts.len() == 4
                && parts[0] == "app"
                && parts[1] == app_id
                && target.arch.as_deref().is_none_or(|arch| parts[2] == arch)
                && target
                    .branch
                    .as_deref()
                    .is_none_or(|branch| parts[3] == branch)
        })
}

fn restrictions(plan: &Value, app_id: &str) -> Vec<Restriction> {
    let mut rows = Vec::new();
    let Some(operations) = plan["operations"].as_array() else {
        return rows;
    };
    let Some(operation) = operations.iter().find(|op| {
        op["ref"]
            .as_str()
            .is_some_and(|r| r.starts_with(&format!("app/{app_id}/")))
    }) else {
        return rows;
    };
    let context = operation["metadata"]["Context"]
        .as_object()
        .cloned()
        .unwrap_or_default();
    // Edits are deliberate extra restrictions, not a fabricated effective
    // context. Full unknown context and inherited policy stay visible below.
    for (category, values) in &context {
        let Some(values) = values.as_str() else {
            continue;
        };
        for value in values
            .split(';')
            .filter(|value| !value.is_empty() && !value.starts_with('!'))
        {
            if value.contains('\\') {
                continue;
            } // GLib escaping stays in the raw review.
            let supported = match category.as_str() {
                "shared" => matches!(value, "network" | "ipc"),
                "sockets" => matches!(
                    value,
                    "x11"
                        | "fallback-x11"
                        | "wayland"
                        | "pulseaudio"
                        | "session-bus"
                        | "system-bus"
                        | "ssh-auth"
                        | "pcsc"
                        | "cups"
                        | "gpg-agent"
                ),
                "devices" => matches!(value, "all" | "dri" | "kvm" | "shm" | "input" | "usb"),
                "features" => matches!(
                    value,
                    "devel" | "multiarch" | "bluetooth" | "canbus" | "per-app-dev-shm"
                ),
                "filesystems" => {
                    !value.starts_with('-')
                        && (!value.contains(':')
                            || value.ends_with(":ro")
                            || value.ends_with(":rw")
                            || value.ends_with(":create"))
                }
                _ => false,
            };
            if supported && rows.len() < 64 {
                // nofilesystem revokes a path, not its declared grant mode.
                let edit_value = if category == "filesystems" {
                    value
                        .strip_suffix(":ro")
                        .or_else(|| value.strip_suffix(":rw"))
                        .or_else(|| value.strip_suffix(":create"))
                        .unwrap_or(value)
                } else {
                    value
                };
                rows.push(Restriction {
                    label: permission_label(category, value),
                    category: category.clone(),
                    value: edit_value.into(),
                    blocked: false,
                });
            }
        }
    }
    for (group, category) in [
        ("Session Bus Policy", "session_bus"),
        ("System Bus Policy", "system_bus"),
    ] {
        if let Some(policy) = operation["metadata"][group].as_object() {
            for (name, access) in policy {
                if rows.len() < 64 && matches!(access.as_str(), Some("talk" | "own")) {
                    rows.push(Restriction {
                        label: crate::i18n::format(
                            crate::i18n::localizer(),
                            "Block {category}: {name}",
                            &[("category", &(category)), ("name", &(name))],
                        ),
                        category: category.into(),
                        value: format!("{name}={}", access.as_str().unwrap()),
                        blocked: false,
                    });
                }
            }
        }
    }
    rows
}

fn permission_label(category: &str, value: &str) -> String {
    let description = match (category, value) {
        ("shared", "network") => "network access",
        ("shared", "ipc") => "shared process communication",
        ("sockets", "wayland") => crate::i18n::label("Wayland display"),
        ("sockets", "x11" | "fallback-x11") => "X11 display access",
        ("sockets", "pulseaudio") => "direct audio access",
        ("sockets", "session-bus") => "unrestricted session bus",
        ("sockets", "system-bus") => "unrestricted system bus",
        ("sockets", "ssh-auth") => "SSH authentication agent",
        ("devices", "all") => "all device access",
        ("devices", "dri") => "graphics acceleration",
        _ => {
            return crate::i18n::format(
                crate::i18n::localizer(),
                "Block {category}: {value}",
                &[("category", &(category)), ("value", &(value))],
            );
        }
    };
    crate::i18n::format(
        crate::i18n::localizer(),
        "Block {description}",
        &[("description", &(description))],
    )
}

/// The consent dialog's heading: a question naming the action and the app.
pub(crate) fn consent_title(kind: OpKind, name: &str) -> String {
    match kind {
        OpKind::Update => crate::i18n::format(
            crate::i18n::localizer(),
            "Update {name}?",
            &[("name", &(name))],
        ),
        _ => crate::i18n::format(
            crate::i18n::localizer(),
            "Install {name}?",
            &[("name", &(name))],
        ),
    }
}

/// How many packages the resolved plan deploys, in words.
pub(crate) fn package_count_label(count: usize) -> String {
    if count == 1 {
        "1 package".to_owned()
    } else {
        format!("{count} packages")
    }
}

/// Explains the restriction switches once, above them, instead of repeating
/// the same subtitle under every row.
pub(crate) const RESTRICTIONS_EXPLAINER: &str = "Turn a switch on to block that access. Features that need it may stop working; everything else keeps your current policy.";

pub(crate) fn present<'a>(
    store: &'a Store,
    size: iced::Size,
    background: Element<'a, Message>,
) -> Element<'a, Message> {
    use crate::ui::chrome;
    use enzui_core::semantics::HeadingLevel;
    use enzui_theme::spacing::SpaceToken;
    use enzui_widgets::banner::{BannerTone, enz_banner};
    use enzui_widgets::capped_dialog_body::CappedDialogBody;
    use enzui_widgets::enz_text::{TextSize, TextTone, enz_text};
    use enzui_widgets::modal_host::enz_modal_host;
    use enzui_widgets::modal_surface::{ModalSurfaceTokens, enz_modal_surface};
    use enzui_widgets::scroll_view::{Axis, enz_scroll_view};
    use enzui_widgets::{ButtonKind, enz_button};
    use iced::widget::{Space, column, row};

    let consent = store.install_consent.as_ref().expect("active consent");
    let update = consent.kind == OpKind::Update;
    let title = enz_text(consent_title(consent.kind, &consent.name))
        .size(TextSize::Title3)
        .weight(iced::font::Weight::Semibold)
        .heading(HeadingLevel::Page);
    let muted = |text: String| {
        enz_text(text)
            .size(TextSize::Footnote)
            .tone(TextTone::Muted)
    };

    let mut body = column![muted(if update {
        crate::i18n::label("Check what this update can access before it is applied.").to_owned()
    } else {
        crate::i18n::label("Check what this app can access before it is installed.").to_owned()
    })]
    .spacing(SpaceToken::Md.px())
    .width(Length::Fill);

    if let Some(error) = &consent.error {
        body = body.push(
            chrome::failure_banner(
                crate::i18n::label("The permission review could not be completed"),
                Some(error.clone()),
            )
            .into_element(),
        );
    } else if let Some(plan) = &consent.plan {
        // Trust warnings lead: they qualify everything below them.
        if plan["remote"]["gpg_verify"] == false
            || plan["operations"].as_array().is_some_and(|ops| {
                ops.iter()
                    .any(|op| op["remote_identity"]["gpg_verify"] == false)
            })
        {
            body = body.push(
                enz_banner(crate::i18n::label("Unsigned source"))
                    .body(crate::i18n::label("One or more package sources do not verify signatures. Install only if you trust these sources."))
                    .tone(BannerTone::Warning)
                    .dismissible(false)
                    .into_element(),
            );
        }

        if update {
            let mut changes = column![
                chrome::section_label(crate::i18n::label("New access in this update")),
                muted(crate::i18n::label("This update changes requested access. Your existing restrictions remain in place.").to_owned()),
            ]
            .spacing(SpaceToken::Xs.px());
            if let Some(additions) = plan["permission_additions"].as_array() {
                for change in additions {
                    changes = changes.push(
                        enz_text(format!(
                            "{} · {}: {}",
                            change["group"]
                                .as_str()
                                .unwrap_or(crate::i18n::label("Policy")),
                            change["key"].as_str().unwrap_or(""),
                            change["requested"]
                                .as_str()
                                .unwrap_or(crate::i18n::label("Removed policy restriction"))
                        ))
                        .size(TextSize::Footnote),
                    );
                }
            }
            body = body.push(changes);
        }

        let count = plan["operations"].as_array().map_or(0, Vec::len);
        let mut source = column![
            chrome::section_label(crate::i18n::label("Source")),
            enz_text(format!(
                "{} · {}",
                consent.remote,
                package_count_label(count)
            ))
            .size(TextSize::Footnote),
        ]
        .spacing(SpaceToken::Xs.px());
        if let Some(url) = plan["remote"]["url"].as_str() {
            source = source.push(muted(source_disclosure(crate::i18n::label("Source"), url)));
        }
        body = body.push(source);

        if !consent.restrictions.is_empty() {
            let mut access = column![
                chrome::section_label(crate::i18n::label("Requested access")),
                muted(RESTRICTIONS_EXPLAINER.to_owned()),
            ]
            .spacing(SpaceToken::Xs.px());
            for (index, restriction) in consent.restrictions.iter().enumerate() {
                access = access.push(
                    enzui_widgets::preferences_row::enz_preferences_row(restriction.label.as_str())
                        .switch(restriction.blocked, move |blocked| {
                            Message::InstallConsentBlock(index, blocked)
                        })
                        .into_element(),
                );
            }
            body = body.push(access);
        }
        body = body.push(muted(crate::i18n::label("Existing user policies remain in place. Portal requests are handled when a feature is used. Broad static access can bypass portal prompts.").to_owned()));
        body = body.push(
            enz_button(
                if consent.details {
                    crate::i18n::label("Hide full permissions")
                } else {
                    crate::i18n::label("Show full permissions and policy sources")
                },
                Message::InstallConsentDetails,
            )
            .kind(ButtonKind::Ghost)
            .icon(if consent.details {
                "chevron-up"
            } else {
                "chevron-down"
            }),
        );
        if consent.details {
            for operation in plan["operations"].as_array().expect("validated operations") {
                body = body.push(enz_text(operation["ref"].as_str().unwrap_or("").to_owned()));
                if let Some(url) = operation["remote_identity"]["url"].as_str() {
                    body = body.push(
                        enz_text(source_disclosure(crate::i18n::label("Package source"), url))
                            .size(TextSize::Footnote),
                    );
                }
                body = body.push(
                    enz_text(
                        serde_json::to_string_pretty(&operation["metadata"]).unwrap_or_default(),
                    )
                    .size(TextSize::Footnote),
                );
            }
            body = body.push(
                enz_text(
                    serde_json::to_string_pretty(&plan["override_sources"]).unwrap_or_default(),
                )
                .size(TextSize::Footnote),
            );
        }
    } else {
        body = body.push(
            row![
                enzui_widgets::spinner::enz_spinner(),
                muted(crate::i18n::format(
                    crate::i18n::localizer(),
                    "Checking packages and permissions for {}…",
                    &[("arg0", &(consent.name))]
                )),
            ]
            .spacing(SpaceToken::Sm.px())
            .align_y(iced::Alignment::Center),
        );
    }
    let actions = row![
        enz_button(crate::i18n::label("Cancel"), Message::InstallConsentCancel)
            .kind(ButtonKind::Quiet),
        Space::new().width(Length::Fill),
        enz_button(
            if update {
                crate::i18n::label("Update")
            } else {
                crate::i18n::label("Install")
            },
            Message::InstallConsentConfirm
        )
        // Accent, not Primary: on the modal surface Primary's fill is
        // nearly the surface colour and the confirm stopped reading as
        // the default action.
        .kind(ButtonKind::Accent)
        .icon(if update {
            "arrow-down-to-line"
        } else {
            "download"
        })
        .enabled(consent.plan.is_some() && consent.error.is_none())
    ]
    .spacing(SpaceToken::Md.px());
    let theme = store.theme_ref();
    let stack = CappedDialogBody::new(
        vec![title.into()],
        enz_scroll_view(body)
            .axis(Axis::Vertical)
            .width(Length::Fill)
            .height(Length::Shrink),
        vec![actions.into()],
        SpaceToken::Md.px(),
        (size.height - 48.0).max(160.0),
    );
    let surface = enz_modal_surface(stack.into())
        .padding(theme.spacing.lg())
        .width(Length::Fixed((size.width - 40.0).clamp(240.0, 620.0)))
        .tokens(ModalSurfaceTokens::from_palette(&theme.palette))
        .into_element();
    enz_modal_host(background, Some(surface), Message::InstallConsentHost)
        .scoped_to(&consent.focus)
        .label(crate::i18n::label("Installation permissions"))
        .into_element()
}

fn source_disclosure(label: &str, url: &str) -> String {
    format!("{label}: {}", services::redact_url_credentials(url))
}

#[cfg(test)]
pub(crate) fn approve_fixture(store: &mut Store) {
    let Some(consent) = store.install_consent.as_ref() else {
        return;
    };
    let generation = consent.generation;
    let update = consent.kind == OpKind::Update;
    let (arch, branch) = (
        consent.target.arch.as_deref().unwrap_or("x86_64"),
        consent.target.branch.as_deref().unwrap_or("stable"),
    );
    let mut plan = json!({"schema_version":1,"review_token":"a".repeat(64),"remote":{"name":consent.remote},
        "operations":[{"type":"install","ref":format!("app/{}/{arch}/{branch}",consent.app_id),"commit":"b".repeat(64),"metadata":{"Context":{"shared":"network;","filesystems":"home:ro;","devices":"dri;","sockets":"wayland;"},"Session Bus Policy":{"org.example.Service":"talk"},"System Bus Policy":{"org.example.System":"talk"}}}],"override_sources":[]});
    if update {
        plan["action"] = json!("update");
        plan["installed"] =
            json!({"ref":format!("app/{}/x86_64/stable",consent.app_id),"commit":"b".repeat(64)});
        plan["requires_consent"] = json!(true);
        plan["permission_additions"] = json!([]);
    }
    let rows = restrictions(&plan, &consent.app_id);
    for category in [
        "shared",
        "filesystems",
        "devices",
        "sockets",
        "session_bus",
        "system_bus",
    ] {
        assert!(
            rows.iter().any(|row| row.category == category),
            "missing {category}"
        );
    }
    let _ = store.update(Message::InstallReviewLoaded(generation, Ok(plan)));
    let _ = store.update(Message::InstallConsentConfirm);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn update_store() -> Store {
        let (mut store, ()) = Store::boot_for_test();
        store.installed = ["org.test.First", "org.test.Second"]
            .into_iter()
            .map(|id| services::AppEntry {
                id: id.into(),
                name: id.into(),
                origin: "flathub".into(),
                install_scope: services::InstallScope::User,
                ..Default::default()
            })
            .collect();
        store.set_updates_known(vec![
            services::AvailableUpdate::user("org.test.First"),
            services::AvailableUpdate::user("org.test.Second"),
        ]);
        store
    }
    fn update_plan(consent: &Consent, added: bool) -> Value {
        json!({"schema_version":1,"action":"update","review_token":"a".repeat(64),"remote":{"name":consent.remote},
            "installed":{"ref":format!("app/{}/x86_64/stable",consent.app_id),"commit":"b".repeat(64)},
            "operations":[{"type":"update","ref":format!("app/{}/x86_64/stable",consent.app_id),"commit":"c".repeat(64),"metadata":{"Context":{"shared":"network;"}}}],
            "requires_consent":added,"permission_additions":[],"override_sources":[]})
    }

    #[test]
    fn without_the_permission_service_updates_start_at_once_without_a_review() {
        let mut store = update_store();
        store.permission_service = false;
        let _ = store.update(Message::UpdateAllRequested);
        // No review dialog: the first update runs straight away, directly,
        // and the second waits its turn.
        assert!(store.install_consent.is_none());
        assert!(
            store.confirmed_install.is_none(),
            "the direct plan was used"
        );
        let running: Vec<_> = store
            .operations
            .iter()
            .filter(|op| !op.finished)
            .map(|op| op.app_id.as_str())
            .collect();
        assert_eq!(running, ["org.test.First"]);
        assert_eq!(store.pending_update_reviews.len(), 1);
    }

    #[test]
    fn cancel_bulk_review_clears_queue_and_late_completion_cannot_update() {
        let mut store = update_store();
        let _ = store.update(Message::UpdateAllRequested);
        let consent = store.install_consent.as_ref().unwrap();
        let generation = consent.generation;
        let plan = update_plan(consent, false);
        assert_eq!(store.pending_update_reviews.len(), 1);
        let _ = store.cancel_install_consent();
        let _ = store.install_review_loaded(generation, Ok(plan));
        assert!(store.operations.is_empty());
        assert!(store.pending_update_reviews.is_empty());
        assert!(store.confirmed_install.is_none());
    }

    #[test]
    fn source_disclosures_hide_credentials_without_changing_review_identity() {
        let plan = json!({"remote":{"url":"https://user:secret@example.invalid/repo"},
            "operations":[{"remote_identity":{"url":"https://token:p@ss@example.invalid/runtime"}}]});
        let original = plan.clone();
        assert_eq!(
            source_disclosure("Source", plan["remote"]["url"].as_str().unwrap()),
            "Source: https://***@example.invalid/repo"
        );
        assert_eq!(
            source_disclosure(
                "Package source",
                plan["operations"][0]["remote_identity"]["url"]
                    .as_str()
                    .unwrap()
            ),
            "Package source: https://***@example.invalid/runtime"
        );
        assert_eq!(plan, original);
        assert_eq!(
            source_disclosure("Source", "https://dl.flathub.org/repo/"),
            "Source: https://dl.flathub.org/repo/"
        );
    }
    fn store() -> Store {
        let (mut store, ()) = Store::boot_for_test();
        store.catalog.popular = vec![services::AppEntry {
            id: "org.test.Consent".into(),
            name: "Consent".into(),
            origin: "flathub".into(),
            ..Default::default()
        }];
        store.refresh_projection();
        store
    }
    #[test]
    fn unresolved_review_cannot_install_and_dismissal_never_confirms() {
        for event in [
            ModalHostEvent::Dismissed(DismissCause::Escape),
            ModalHostEvent::Dismissed(DismissCause::Scrim),
        ] {
            let mut store = store();
            let _ = store.update(Message::InstallRequested(
                "org.test.Consent".into(),
                "Consent".into(),
            ));
            assert!(store.install_consent.is_some());
            assert!(store.operations.is_empty());
            let _ = store.update(Message::InstallConsentConfirm);
            assert!(store.operations.is_empty());
            let _ = store.update(Message::InstallConsentHost(event));
            assert!(store.install_consent.is_none());
            assert!(store.confirmed_install.is_none());
            assert!(store.operations.is_empty());
        }
    }

    #[test]
    fn explicit_review_confirmation_starts_only_the_named_install() {
        let mut store = store();
        let _ = store.update(Message::InstallRequested(
            "org.test.Consent".into(),
            "Consent".into(),
        ));
        approve_fixture(&mut store);
        assert!(store.install_consent.is_none());
        assert_eq!(store.operations.len(), 1);
        assert_eq!(store.operations[0].app_id, "org.test.Consent");
        assert!(store.confirmed_install.is_none());
    }
}

#[cfg(test)]
mod policy_settlement_tests {
    use super::*;
    #[test]
    fn late_policy_result_cannot_modify_another_or_reopened_page() {
        let (mut store, ()) = Store::boot_for_test();
        store.route = crate::Route::Detail;
        store.detail_app_id = Some("org.test.Current".into());
        store.detail_generation = 5;
        let _ = store.open_sandbox_dialog();
        store.permission_toggles =
            crate::PermissionTogglesState::Ready(vec![services::PermissionToggle {
                key: "network".into(),
                label: "Network".into(),
                category: "network".into(),
                app_default: true,
                enabled: true,
            }]);
        for (app_id, generation) in [("org.test.Other", 5), ("org.test.Current", 4)] {
            let _ = store.update(Message::OwnedPermissionMutationFinished {
                app_id: app_id.into(),
                generation,
                completion: Box::new(Message::PermissionOverrideFinished {
                    key: "network".into(),
                    attempted: true,
                    previous: false,
                    result: Err(services::PermissionActionFailure::unavailable(
                        services::PermissionAction::Write,
                    )),
                }),
            });
            assert!(store.permission_toggles()[0].enabled);
            assert!(store.permission_error().is_none());
        }
    }
}
