//! Catalogue geometry: row pitch, overscan, page inset, and the
//! hairline divider.

use super::ROW_HEIGHT;
use enzui_theme::spacing::SpaceToken;
use iced::Element;

// Virtual-list item height must equal rendered row height plus its gap.
pub(crate) fn row_pitch() -> f32 {
    ROW_HEIGHT + ROW_GAP.px()
}

// The geometry cell trails live scrolling by one frame, so catalogue lists
// retain this much content beyond each viewport edge.
const CATALOG_PRERENDER_PX: f32 = 600.0;

/// Returns the catalogue prerender margin in whole rows.
pub(crate) fn catalog_overscan_rows() -> usize {
    (CATALOG_PRERENDER_PX / row_pitch()).ceil() as usize
}

/// The gap between virtualized rows, included in row pitch.
pub(crate) const ROW_GAP: SpaceToken = SpaceToken::Xs;

/// The shared scroll-body inset, resolved from the framework large spacing token.
pub(crate) fn page_inset() -> f32 {
    SpaceToken::Lg.px()
}

pub(crate) const ROW_ICON_PX: f32 = 40.0;

/// A full-width framework divider with renderer-scale pixel snapping.
pub(crate) fn hairline<'a, Message: 'a>() -> Element<'a, Message> {
    enzui_widgets::divider::enz_divider().into_element()
}
