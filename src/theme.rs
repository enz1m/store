//! Palette accessor used by Store style closures.

use enzui_theme::Palette;

/// The palette for the runtime `iced::Theme`, as every style closure sees it.
pub(crate) fn palette(theme: &iced::Theme) -> Palette {
    Palette::from_iced(theme)
}
