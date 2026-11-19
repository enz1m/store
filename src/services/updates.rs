// Available updates.

/// Keep the reporting installation attached to updates; display deduplication must not retarget mutations.
#[derive(Clone, Debug, Eq, Hash, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct AvailableUpdate {
    /// The application id, as `remote-ls --updates --columns=application`
    /// printed it.
    pub app_id: String,
    /// The installations whose update query reported this id, in query order
    /// (user first). Never empty: an `AvailableUpdate` exists because some
    /// scope reported it.
    pub scopes: Vec<InstallScope>,
}

impl AvailableUpdate {
    /// An update reported by the per-user installation.
    #[must_use]
    pub(crate) fn user(app_id: impl Into<String>) -> Self {
        Self::in_scope(app_id, InstallScope::User)
    }

    /// An update reported by the default system-wide installation.
    #[must_use]
    #[cfg(test)]
    pub(crate) fn system(app_id: impl Into<String>) -> Self {
        Self::in_scope(app_id, InstallScope::System)
    }

    /// An update reported by exactly one named installation.
    #[must_use]
    pub(crate) fn in_scope(app_id: impl Into<String>, scope: InstallScope) -> Self {
        Self {
            app_id: app_id.into(),
            scopes: vec![scope],
        }
    }

    /// Record that another installation also reported this id, keeping query
    /// order and never duplicating a scope.
    pub(crate) fn also_reported_in(&mut self, scope: InstallScope) {
        if !self.scopes.contains(&scope) {
            self.scopes.push(scope);
        }
    }

    /// Whether the **user-scope** update query positively reported this id.
    ///
    /// The whole authorization turns on this, and on nothing else. "Installed
    /// in the user scope" is a different question with a different answer: an
    /// app can be installed in both and be behind in only one.
    #[must_use]
    pub(crate) fn reported_by_user_scope(&self) -> bool {
        self.scopes.contains(&InstallScope::User)
    }

    /// Report whether the given installation contributed this update.
    #[must_use]
    pub(crate) fn reported_in(&self, scope: &InstallScope) -> bool {
        self.scopes.contains(scope)
    }

    /// Whether this update may be applied, and in which installation.
    ///
    /// `Allowed(USER)` **only** when the user-scope query reported it.
    /// Otherwise [`SYSTEM_MANAGED`]: the update is real, it is visible, and
    /// this build cannot apply it. There is deliberately no arm that resolves
    /// a system-only update to `--user`.
    #[must_use]
    #[cfg(test)]
    pub(crate) fn authorization(&self) -> Authorization {
        if self.reported_by_user_scope() {
            Authorization::Allowed(MutationScope::USER)
        } else {
            Authorization::Refused(SYSTEM_MANAGED)
        }
    }

    /// Whether this update may be applied to an app whose actionable copy is
    /// `installed` (the copy Update and Remove address: the user copy when
    /// there is one).
    ///
    /// A user-reported update applies in the user installation, as
    /// [`authorization`](Self::authorization) says. A system-reported update
    /// applies in the default system installation only when that is the
    /// app's only copy (preinstalled apps): with a user copy present the
    /// action addresses the user copy, and a system-only report must not be
    /// applied there. Named installations stay refused.
    #[must_use]
    pub(crate) fn authorization_for(&self, installed: &InstallScope) -> Authorization {
        if self.reported_by_user_scope() {
            Authorization::Allowed(MutationScope::USER)
        } else if *installed == InstallScope::System && self.reported_in(&InstallScope::System) {
            Authorization::Allowed(MutationScope::SYSTEM)
        } else {
            Authorization::Refused(SYSTEM_MANAGED)
        }
    }

    /// Drop the installation a completed operation just settled, reporting
    /// whether anything is still pending.
    ///
    /// A finished update settles exactly one installation — the one it ran in
    /// — so an app that was behind in both scopes is still behind in the other
    /// afterwards, and its row must say so until the authoritative re-read
    /// lands. Returns `false` when nothing remains, which is the caller's cue
    /// to drop the whole entry.
    #[must_use]
    pub(crate) fn settle(&mut self, scope: &InstallScope) -> bool {
        self.scopes.retain(|reported| reported != scope);
        !self.scopes.is_empty()
    }
}
