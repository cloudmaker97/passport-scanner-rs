//! Links to where the project lives, shown beside the version.
use crate::Message;
use iced::widget::{row, svg, text};
use iced::{Alignment, Color, Element};
use std::sync::LazyLock;

/// The author's website.
pub(crate) const WEBSITE: &str = "https://dennis-heinri.ch";
/// The source code and its releases.
pub(crate) const REPOSITORY: &str = "https://github.com/cloudmaker97/passport-scanner-rs";

/// Octicons (MIT, see licenses/octicons-MIT.txt), drawn in the text colour.
static GLOBE: LazyLock<svg::Handle> =
    LazyLock::new(|| svg::Handle::from_memory(include_bytes!("../assets/globe.svg").as_slice()));
static GITHUB: LazyLock<svg::Handle> =
    LazyLock::new(|| svg::Handle::from_memory(include_bytes!("../assets/github.svg").as_slice()));

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Link {
    Website,
    Repository,
}

impl Link {
    pub(crate) const fn url(self) -> &'static str {
        match self {
            Self::Website => WEBSITE,
            Self::Repository => REPOSITORY,
        }
    }
    fn icon(self) -> svg::Handle {
        match self {
            Self::Website => GLOBE.clone(),
            Self::Repository => GITHUB.clone(),
        }
    }
}

/// A quiet button with the link's icon and `label`, opening it in the browser.
pub(crate) fn button<'a>(link: Link, label: &'a str, ink: Color) -> Element<'a, Message> {
    iced::widget::button(
        row![
            svg(link.icon())
                .width(14)
                .height(14)
                .style(move |_, _| svg::Style { color: Some(ink) }),
            text(label).size(12),
        ]
        .spacing(6)
        .align_y(Alignment::Center),
    )
    .style(crate::view::quiet)
    .on_press(Message::OpenLink(link.url()))
    .into()
}
