//! Update availability and install scope.

use super::*;

impl Store {
    pub(crate) fn update_for(&self, app_id: &str) -> Option<&services::AvailableUpdate> {
        self.updates_available()
            .iter()
            .find(|update| update.app_id == app_id)
    }

    // Empty results can mean loading or failure; only claims_up_to_date establishes currency.
    pub(crate) fn updates_available(&self) -> &[services::AvailableUpdate] {
        self.update_discovery.updates()
    }

    // Delivers a complete two-scope update check through the reducer.
    #[cfg(test)]
    pub(crate) fn deliver_known_updates(&mut self, updates: Vec<services::AvailableUpdate>) {
        let generation = self.updates_generation;
        let _ = self.update(Message::UpdatesLoaded {
            generation,
            scan: Box::new(services::UpdateScan::complete(updates)),
        });
    }

    #[cfg(test)]
    pub(crate) fn set_updates_known(&mut self, updates: Vec<services::AvailableUpdate>) {
        self.update_discovery = services::UpdateDiscovery::Known {
            updates,
            last_checked_ms: 0,
        };
    }

    // The pending-update ids, for the row builders that only need to know
    // *whether* a row is behind.
    //
    // Deduplicated by construction: `updates_available` holds one entry per
    // id whatever the scopes reported it, which is the documented rendering
    // policy — a user with one out-of-date app has one thing to do.
    pub(crate) fn update_ids(&self) -> Vec<String> {
        self.updates_available()
            .iter()
            .map(|update| update.app_id.clone())
            .collect()
    }

    // Authorize updates from the scope that reported them, not the preferred installed copy.
    pub(crate) fn update_authorization(&self, app_id: &str) -> services::Authorization {
        match self.update_for(app_id) {
            Some(update) => update.authorization_for(&self.installed_scope(app_id)),
            None => self.installed_scope(app_id).mutation(),
        }
    }

    // Use all installed rows so confirmation can disclose surviving system copies.
    pub(crate) fn system_copy_survives_removal(&self, app_id: &str) -> bool {
        let removable = self.installed_scope(app_id) == services::InstallScope::User;
        removable
            && self
                .installed
                .iter()
                .any(|e| e.id == app_id && e.install_scope.is_system_managed())
    }

    // Prefer the mutable user installation; missing scope remains an explicit refusal.
    pub(crate) fn installed_scope(&self, app_id: &str) -> services::InstallScope {
        let mut first = services::InstallScope::Unknown;
        let mut seen = false;
        for entry in self.installed.iter().filter(|entry| entry.id == app_id) {
            if entry.install_scope == services::InstallScope::User {
                return services::InstallScope::User;
            }
            if !seen {
                first = entry.install_scope.clone();
                seen = true;
            }
        }
        first
    }

    // Fresh installs resolve the origin against positively known repositories, not an absent installation.
    pub(crate) fn install_authorization(&self, app_id: &str) -> services::Authorization {
        let origin = self
            .installed
            .iter()
            .find(|entry| entry.id == app_id)
            .map(|entry| entry.origin.clone())
            .or_else(|| self.app_entry(app_id).map(|entry| entry.origin));
        match (origin, self.removed_origin(app_id)) {
            (None, Some(removed)) => {
                services::removed_origin_authorization(removed, &self.remotes_load, &self.remotes)
            }
            (origin, _) => services::install_authorization(
                &origin.unwrap_or_default(),
                &self.remotes_load,
                &self.remotes,
            ),
        }
    }

    // Consent and permission reads resolve a fresh install's repository the same way.
    pub(crate) fn install_remote(&self, app_id: &str) -> Option<&services::RemoteInfo> {
        let origin = self
            .installed
            .iter()
            .find(|entry| entry.id == app_id && entry.install_scope == services::InstallScope::User)
            .map(|entry| entry.origin.clone())
            .or_else(|| self.app_entry(app_id).map(|entry| entry.origin));
        match origin {
            Some(origin) => self.remotes.iter().find(|remote| {
                services::InstallScope::from_remote_options(&remote.options)
                    == services::InstallScope::User
                    && services::matches_catalog_origin(remote, &origin)
            }),
            None => self
                .removed_origin(app_id)
                .filter(|removed| removed.scope == services::InstallScope::User)
                .and_then(|removed| removed.current_remote(&self.remotes)),
        }
    }

    // An origin only a system repository carries is still described by that repository.
    pub(crate) fn metadata_remote(&self, app_id: &str) -> Option<services::MetadataRemote> {
        if let Some(remote) = self.install_remote(app_id) {
            return Some(services::MetadataRemote {
                name: remote.name.clone(),
                system: false,
            });
        }
        let origin = self.app_entry(app_id)?.origin;
        self.remotes
            .iter()
            .find(|remote| {
                services::InstallScope::from_remote_options(&remote.options)
                    == services::InstallScope::System
                    && services::matches_catalog_origin(remote, &origin)
            })
            .map(|remote| services::MetadataRemote {
                name: remote.name.clone(),
                system: true,
            })
    }

    // A source recorded at removal applies only when neither an installed row nor the catalogue names one.
    pub(crate) fn removed_origin(&self, app_id: &str) -> Option<&services::RemovedOrigin> {
        self.recently_uninstalled
            .iter()
            .find(|entry| entry.id == app_id)
            .and_then(|entry| entry.removed_origin.as_ref())
    }

    pub(crate) fn repository_answer(&self) -> services::RepositoryAnswer<'_> {
        services::RepositoryAnswer {
            load: &self.remotes_load,
            remotes: &self.remotes,
        }
    }

    pub(crate) fn repositories_error(&self) -> Option<&str> {
        self.remotes_load.error()
    }

    // View and reducer share authorization: installs use repository evidence, other actions use installation evidence.
    pub(crate) fn action_authorization(
        &self,
        app_id: &str,
        kind: OpKind,
    ) -> services::Authorization {
        match kind {
            OpKind::Install => self.install_authorization(app_id),

            OpKind::Update => self.update_authorization(app_id),
            OpKind::Uninstall => self.installed_scope(app_id).mutation(),
        }
    }
}
