//! Browse categories.

// ---------------------------------------------------------------------------
// Categories  (ported from round one)
// ---------------------------------------------------------------------------

/// Every XDG main-category key the Categories view offers, in display order.
///
/// This is the **single table** both the category sidebar/grid and the filter
/// logic read from. Adding a row here is all that is needed to surface a new
/// category — no second list, no drift.
pub(crate) const BROWSE_CATEGORIES: &[&str] = &[
    "audio",
    "audiovideo",
    "business",
    "development",
    "education",
    "entertainment",
    "game",
    "graphics",
    "network",
    "office",
    "productivity",
    "science",
    "social",
    "system",
    "utility",
    "video",
];

/// Human-readable label for a category key. Unknown keys fall through
/// title-cased rather than panic — an app tagged with a category we didn't
/// anticipate still gets a chip.
pub(crate) fn browse_category_label(key: &str) -> String {
    match key {
        "game" => crate::i18n::label("Games").to_owned(),
        "utility" => crate::i18n::label("Utilities").to_owned(),
        "network" => crate::i18n::label("Internet").to_owned(),
        "audiovideo" => crate::i18n::label("Audio & Video").to_owned(),
        "graphics" => crate::i18n::label("Graphics & Design").to_owned(),
        "office" => crate::i18n::label("Office").to_owned(),
        "development" => crate::i18n::label("Developer Tools").to_owned(),
        "education" => crate::i18n::label("Education").to_owned(),
        "science" => crate::i18n::label("Science").to_owned(),
        "system" => crate::i18n::label("System").to_owned(),
        "settings" => crate::i18n::label("Settings").to_owned(),
        "business" => crate::i18n::label("Business").to_owned(),
        "entertainment" => crate::i18n::label("Entertainment").to_owned(),
        "audio" => crate::i18n::label("Music & Audio").to_owned(),
        "video" => crate::i18n::label("Photo & Video").to_owned(),
        "productivity" => crate::i18n::label("Productivity").to_owned(),
        "social" => crate::i18n::label("Social & News").to_owned(),
        "" => String::new(),
        other => {
            let mut chars = other.chars();
            match chars.next() {
                Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        }
    }
}

/// Pluralise a count of apps: `1 app`, `0 apps`, `13 apps`.
///
/// Trivial, but it was wrong on screen ("1 apps" in `shots-cats`), and a store
/// with a one-app category is a normal state rather than an edge case.
pub(crate) fn app_count_label(n: usize) -> String {
    crate::i18n::count("app-app-count", n)
}

/// "1 update" / "N updates" for the Library's pending-updates band.
pub(crate) fn update_count_label(n: usize) -> String {
    crate::i18n::count("app-update-count", n)
}

/// "1 task" / "N tasks" for running install, update and removal work.
pub(crate) fn task_count_label(n: usize) -> String {
    crate::i18n::count("app-task-count", n)
}

/// Lucide icon for a category key.
pub(crate) fn browse_category_icon(key: &str) -> &'static str {
    match key {
        "game" => "gamepad-2",
        "utility" => "wrench",
        "network" => "globe",
        "audiovideo" => "clapperboard",
        "graphics" => "palette",
        "office" => "briefcase",
        "development" => "code",
        "education" => "graduation-cap",
        "science" => "flask-conical",
        "system" => "settings",
        "business" => "briefcase-business",
        "entertainment" => "tv",
        "music" => "music",
        "audio" => "music",
        "video" => "clapperboard",
        "productivity" => "list-checks",
        "social" => "newspaper",
        _ => "package",
    }
}
