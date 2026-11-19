//! Permission chips and defaults.

/// Effective access, with an explanation when the app never requested it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PermissionDefault {
    /// The app's metadata declares this permission.
    Granted,
    /// The permission is absent from the app's metadata.
    NotRequested,
    /// An override has explicitly denied this permission.
    Denied,
}

impl PermissionDefault {
    /// An enabled override can grant access absent from declared metadata.
    pub(crate) fn from_toggle(app_default: bool, enabled: bool) -> Self {
        match (app_default, enabled) {
            (_, true) => PermissionDefault::Granted,
            (true, false) => PermissionDefault::Denied,
            (false, false) => PermissionDefault::NotRequested,
        }
    }

    /// The user-facing label for a permission toggle row.
    pub(crate) fn label(self) -> &'static str {
        match self {
            PermissionDefault::Granted => crate::i18n::label("Current access: allowed"),
            PermissionDefault::NotRequested => {
                crate::i18n::label("Not requested; currently blocked")
            }
            PermissionDefault::Denied => crate::i18n::label("Current access: blocked"),
        }
    }
}
