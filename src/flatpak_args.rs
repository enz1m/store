//! Validation and argument construction for values passed to Flatpak.
//!
//! Catalogue, cache, and CLI output are all external input.  Every Flatpak
//! effect validates those values again, then places a single `--` before the
//! first operand so an operand can never be reinterpreted as an option.

/// Flatpak's reverse-DNS application-ID grammar.
///
/// IDs have at least three non-empty components, are at most 255 bytes, and
/// every component starts with an ASCII letter followed by ASCII letters,
/// digits, or underscores.
pub(crate) fn validate_app_id(app_id: &str) -> Result<(), String> {
    if app_id.is_empty() || app_id.len() > 255 {
        return Err("invalid Flatpak application ID".into());
    }
    let mut components = 0;
    for component in app_id.split('.') {
        components += 1;
        let mut bytes = component.bytes();
        if !bytes.next().is_some_and(|byte| byte.is_ascii_alphabetic())
            || !bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        {
            return Err("invalid Flatpak application ID".into());
        }
    }
    if components < 3 {
        return Err("invalid Flatpak application ID".into());
    }
    Ok(())
}

/// Validate a Flatpak remote name before it becomes an operand.
pub(crate) fn validate_remote_name(name: &str) -> Result<(), String> {
    if name.is_empty()
        || name.len() > 255
        || name.starts_with('-')
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
    {
        return Err("invalid Flatpak remote name".into());
    }
    Ok(())
}

/// An argument vector whose operands are protected from option parsing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FlatpakArgs {
    args: Vec<String>,
    operands_started: bool,
}

impl FlatpakArgs {
    pub(crate) fn new(subcommand: &str) -> Self {
        Self {
            args: vec![subcommand.to_owned()],
            operands_started: false,
        }
    }

    pub(crate) fn options<'a>(&mut self, options: impl IntoIterator<Item = &'a str>) {
        debug_assert!(!self.operands_started, "options must precede operands");
        self.args.extend(options.into_iter().map(str::to_owned));
    }

    /// Append caller data as one name=value option, refusing NUL and ASCII control characters.
    ///
    /// Semantic validation of the value remains the caller's responsibility.
    pub(crate) fn option_with_value(&mut self, name: &str, value: &str) {
        debug_assert!(!self.operands_started, "options must precede operands");
        debug_assert!(
            name.starts_with("--"),
            "an option carries its own leading dashes"
        );
        debug_assert!(
            !value.bytes().any(|byte| byte.is_ascii_control()),
            "a control character in an option value is never a legitimate Flatpak argument"
        );
        self.args.push(format!("{name}={value}"));
    }

    /// Append a validated application ID as an operand.
    pub(crate) fn app_id(&mut self, app_id: &str) -> Result<(), String> {
        validate_app_id(app_id)?;
        self.operand(app_id);
        Ok(())
    }

    /// Append a validated remote name as an operand.
    pub(crate) fn remote(&mut self, name: &str) -> Result<(), String> {
        validate_remote_name(name)?;
        self.operand(name);
        Ok(())
    }

    /// Append a non-ID operand (for example a validated repository URL).
    pub(crate) fn literal_operand(&mut self, operand: &str) {
        self.operand(operand);
    }

    fn operand(&mut self, operand: &str) {
        if !self.operands_started {
            self.args.push("--".to_owned());
            self.operands_started = true;
        }
        self.args.push(operand.to_owned());
    }

    pub(crate) fn finish(self) -> Vec<String> {
        self.args
    }
}
