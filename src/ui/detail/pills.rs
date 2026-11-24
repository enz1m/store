//! Permission pills, gallery, and columns.

use super::*;
use iced::widget::column;

/// Badges for declared access, from the permissions dialog's items: each broad
/// permission in a warning tone, then each kind of narrower access.
pub(crate) fn permission_pills<'a>(perms: Option<&Permissions>) -> Element<'a, Message> {
    let Some(p) = perms else {
        return chrome::muted_line(crate::i18n::label("Permissions unavailable"));
    };
    let badges = permission_badges(&permission_groups(p));
    if badges.is_empty() {
        return chrome::muted_line(crate::i18n::label("No sandbox exceptions declared"));
    }
    let items: Vec<Element<'a, Message>> = badges
        .into_iter()
        .map(|(label, broad)| {
            enz_badge(&label)
                .tone(if broad {
                    BadgeTone::Warning
                } else {
                    BadgeTone::Accent
                })
                .into_element()
        })
        .collect();
    iced::widget::Row::with_children(items)
        .spacing(SpaceToken::Sm.px())
        .align_y(Alignment::Center)
        .wrap()
        .into()
}

const GALLERY_STRIP_HEIGHT: f32 = 220.0;
const GALLERY_STRIP_CELL_WIDTH: f32 = 340.0;

pub(crate) fn gallery_image<'a>(
    image: crate::bounded_image::RenderImage,
    alt: String,
) -> Element<'a, Message> {
    use enzui_widgets::image_widget::{EnzImageFit, enz_image_handle};
    enz_image_handle(image.gallery_handle())
        .fit(EnzImageFit::Contain)
        .alt_text(alt)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

pub(crate) fn screenshot_gallery<'a>(store: &'a Store) -> Element<'a, Message> {
    use enzui_widgets::horizontal_scroll_view::enz_horizontal_scroll_view;

    let shots = store.screenshots();
    let total = shots.len();
    let mut strip = row![].spacing(SpaceToken::Sm.px());
    for (index, shot) in shots.iter().enumerate() {
        let caption = shot.caption.trim();
        let alt = if caption.is_empty() {
            crate::i18n::format(
                crate::i18n::localizer(),
                "View screenshot {} of {}",
                &[("arg0", &(index + 1)), ("arg1", &(total))],
            )
        } else {
            crate::i18n::format(
                crate::i18n::localizer(),
                "View screenshot {} of {}: {}",
                &[
                    ("arg0", &(index + 1)),
                    ("arg1", &(total)),
                    ("arg2", &(caption)),
                ],
            )
        };
        let cell = container(gallery_image(shot.image.clone(), alt.clone()))
            .width(Length::Fill)
            .height(Length::Fixed(GALLERY_STRIP_HEIGHT));
        strip = strip.push(
            enz_pressable(cell.into(), Message::OpenScreenshot(index))
                .style(PressableStyle::Plain)
                .aria_label(alt)
                .padding(0.0)
                .radius(SpaceToken::Sm.px())
                .id(Store::screenshot_strip_id(index))
                .width(Length::Fixed(GALLERY_STRIP_CELL_WIDTH)),
        );
    }

    enz_horizontal_scroll_view(strip)
        .strip_height(GALLERY_STRIP_HEIGHT)
        .width(Length::Fill)
        .into_element()
}

/// The gallery's shape while its screenshots load: the same strip of cells
/// at the same height, so the page below does not move when they arrive.
pub(crate) fn screenshot_gallery_placeholder<'a>() -> Element<'a, Message> {
    use enzui_widgets::horizontal_scroll_view::enz_horizontal_scroll_view;
    let mut strip = row![].spacing(SpaceToken::Sm.px());
    for _ in 0..3 {
        strip = strip.push(chrome::skeleton_block(
            Length::Fixed(GALLERY_STRIP_CELL_WIDTH),
            GALLERY_STRIP_HEIGHT,
            super::loading_screenshots_copy(),
        ));
    }
    enz_horizontal_scroll_view(strip)
        .strip_height(GALLERY_STRIP_HEIGHT)
        .width(Length::Fill)
        .into_element()
}

const VIEWER_CLOSE_PX: f32 = 36.0;

pub(crate) fn present_screenshot_viewer<'a>(
    store: &'a Store,
    size: iced::Size,
    background: Element<'a, Message>,
) -> Element<'a, Message> {
    use enzui_widgets::full_screen_overlay::enz_full_screen_overlay;

    let Some(index) = store.screenshot_viewer() else {
        return crate::ui::closed_dialog_host(background);
    };
    let Some(shot) = store.screenshots().get(index) else {
        return crate::ui::closed_dialog_host(background);
    };
    let total = store.screenshots().len();
    let caption = shot.caption.trim();
    let alt = if caption.is_empty() {
        crate::i18n::format(
            crate::i18n::localizer(),
            "Screenshot {} of {}",
            &[("arg0", &(index + 1)), ("arg1", &(total))],
        )
    } else {
        crate::i18n::format(
            crate::i18n::localizer(),
            "Screenshot {} of {}: {}",
            &[
                ("arg0", &(index + 1)),
                ("arg1", &(total)),
                ("arg2", &(caption)),
            ],
        )
    };
    let viewer_width = size.width * 0.85;
    let viewer_height = (size.width * 0.62).min(size.height * 0.85);
    let image = container(gallery_image(shot.image.clone(), alt.clone()))
        .width(Length::Fixed(viewer_width))
        .height(Length::Fixed(viewer_height));
    // The close button floats over the screenshot's top-right corner, where
    // the eye already is; the overlay's own sits in the window's corner, on
    // top of the window's close button. It carries the overlay's close id,
    // which opening the viewer focuses.
    let close = viewer_button(
        "x",
        Message::ScreenshotViewerEvent(
            enzui_widgets::full_screen_overlay::FullScreenOverlayEvent::Dismissed(
                enzui_core::modal::DismissCause::CloseButton,
            ),
        ),
        crate::i18n::label("Close"),
    )
    .id(enzui_core::identity::derive_id(
        enzui_widgets::full_screen_overlay::DEFAULT_ID_PREFIX,
        "close",
    ))
    .into_element();
    let mut framed = iced::widget::stack![
        image,
        container(close)
            .width(Length::Fixed(viewer_width))
            .align_x(Alignment::End)
            .padding(SpaceToken::Md.px()),
    ];
    // The previous and next screenshots, at the image's middle edges, where
    // there is one to go to.
    if total > 1 {
        let side = |target: Option<usize>,
                    icon: &'static str,
                    label: &'static str|
         -> Element<'a, Message> {
            match target {
                Some(target) => viewer_button(
                    icon,
                    Message::OpenScreenshot(target),
                    crate::i18n::label(label),
                )
                .into_element(),
                None => Space::new().width(Length::Fixed(VIEWER_CLOSE_PX)).into(),
            }
        };
        framed = framed.push(
            container(
                row![
                    side(index.checked_sub(1), "chevron-left", "Previous screenshot"),
                    Space::new().width(Length::Fill),
                    side(
                        (index + 1 < total).then_some(index + 1),
                        "chevron-right",
                        "Next screenshot"
                    ),
                ]
                .align_y(Alignment::Center),
            )
            .width(Length::Fixed(viewer_width))
            .height(Length::Fixed(viewer_height))
            .center_y(Length::Fixed(viewer_height))
            .padding([0.0, SpaceToken::Md.px()]),
        );
    }
    enz_full_screen_overlay(framed.into())
        .show_close_button(false)
        .label(alt)
        .into_host_with(background, Message::ScreenshotViewerEvent)
}

/// A round, dark button floating over a screenshot in the viewer.
fn viewer_button<'a>(
    icon: &'static str,
    message: Message,
    label: &'static str,
) -> enzui_widgets::pressable::EnzPressable<'a, Message> {
    enzui_widgets::pressable::enz_pressable(
        container(
            enzui_widgets::icon::enz_icon(icon)
                .size(20.0)
                .tint(iced::Color::WHITE)
                .into_element(),
        )
        .center_x(Length::Fixed(VIEWER_CLOSE_PX))
        .center_y(Length::Fixed(VIEWER_CLOSE_PX))
        .style(|_| container::Style {
            background: Some(iced::Color::from_rgba(0.0, 0.0, 0.0, 0.6).into()),
            border: iced::Border {
                radius: (VIEWER_CLOSE_PX / 2.0).into(),
                ..iced::Border::default()
            },
            ..container::Style::default()
        })
        .into(),
        message,
    )
    .style(PressableStyle::Plain)
    .padding(0.0)
    .radius(VIEWER_CLOSE_PX / 2.0)
    .width(Length::Shrink)
    .aria_label(label)
}

/// The viewport breakpoint between stacked and paired Detail information blocks.
pub(crate) const DETAIL_TWO_COL_MIN_WIDTH: f32 = 900.0;

// Reserve action width across progress updates to prevent header reflow.
pub(crate) const DETAIL_ACTION_WIDTH: f32 = 132.0;

/// Lay `left` and `right` out side-by-side (each taking half the width) when
/// the viewport is at least [`DETAIL_TWO_COL_MIN_WIDTH`], or stacked in one
/// column below it. No card, no border — just two `FillPortion` halves with a
/// major-region gap, matching the app's flat, card-averse vocabulary.
pub(crate) fn two_column<'a>(
    left: Element<'a, Message>,
    right: Element<'a, Message>,
    width: f32,
) -> Element<'a, Message> {
    if width >= DETAIL_TWO_COL_MIN_WIDTH {
        // Equal halves, each as tall as the taller one.
        enzui_widgets::grid::enz_grid(2, vec![left, right])
            .stretch_cells(true)
            .stretch_rows(true)
            .gap(SpaceToken::Md.px())
            .into_element()
    } else {
        column![left, right].spacing(SpaceToken::Md.px()).into()
    }
}
