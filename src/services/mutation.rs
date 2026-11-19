/// Authorized scope for a mutating Flatpak command.
/// Only authorization produces this value, preventing callers from constructing unchecked scope flags.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct MutationScope(&'static str);

impl MutationScope {
    /// The per-user installation.
    pub(crate) const USER: Self = Self("--user");

    /// The default system-wide installation. Flatpak's system helper performs the
    /// change under polkit: updates are allowed for the active local user, and
    /// removal for administrators (enzimOS desktop accounts are in `wheel`).
    pub(crate) const SYSTEM: Self = Self("--system");

    /// The `flatpak` option that selects this installation.
    #[must_use]
    pub(crate) fn option(self) -> &'static str {
        self.0
    }

    /// Resolve the installation targeted by this authorized mutation scope.
    #[must_use]
    pub(crate) fn installation(self) -> InstallScope {
        match self.0 {
            "--user" => InstallScope::User,
            "--system" => InstallScope::System,
            named => InstallScope::Named(named.trim_start_matches("--installation=").to_owned()),
        }
    }
}
