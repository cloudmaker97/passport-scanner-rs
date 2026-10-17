//! Compact document workspace and modal application settings.
use super::*;
use iced::widget::{
    Column, button, column, container, image, mouse_area, opaque, pick_list, row, rule, scrollable,
    space, stack, text, toggler,
};
use iced::{Alignment, Background, Border, Color, Element, Fill, Theme};
use model::{Verification, verification};

#[derive(Clone, Copy)]
struct Colors {
    ground: Color,
    surface: Color,
    sunk: Color,
    ink: Color,
    muted: Color,
    line: Color,
    good: Color,
    bad: Color,
    /// Background, text and edge of whatever is selected.
    chosen: Color,
    chosen_ink: Color,
    chosen_line: Color,
}
impl Colors {
    fn new(appearance: Appearance) -> Self {
        let rgb = |r, g, b| Color::from_rgb8(r, g, b);
        match appearance {
            Appearance::Dark => Self {
                ground: rgb(13, 15, 17),
                surface: rgb(19, 22, 25),
                sunk: rgb(8, 9, 11),
                ink: rgb(231, 233, 235),
                muted: rgb(157, 163, 169),
                line: rgb(38, 42, 46),
                good: rgb(95, 185, 124),
                bad: rgb(228, 105, 90),
                chosen: rgb(231, 233, 235),
                chosen_ink: rgb(13, 15, 17),
                chosen_line: rgb(231, 233, 235),
            },
            Appearance::Light => Self {
                ground: Color::WHITE,
                surface: Color::WHITE,
                sunk: rgb(244, 245, 246),
                ink: rgb(21, 24, 27),
                muted: rgb(83, 88, 94),
                line: rgb(220, 222, 225),
                good: rgb(31, 107, 54),
                bad: rgb(168, 27, 16),
                // A full black block outshouts everything on a white page and
                // swallows the green and red marks; a light grey still reads
                // as chosen.
                chosen: rgb(222, 226, 231),
                chosen_ink: rgb(21, 24, 27),
                chosen_line: rgb(176, 182, 189),
            },
        }
    }
    fn theme(theme: &Theme) -> Self {
        Self::new(if theme.palette().background.r < 0.5 {
            Appearance::Dark
        } else {
            Appearance::Light
        })
    }
}

/// A pick-list entry labelled in the current language.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Choice<T> {
    value: T,
    label: &'static str,
}
impl<T> std::fmt::Display for Choice<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label)
    }
}

impl App {
    pub fn theme(&self) -> Theme {
        let c = Colors::new(self.appearance);
        Theme::custom(
            TITLE,
            iced::theme::Palette {
                background: c.ground,
                text: c.ink,
                primary: c.ink,
                success: c.good,
                danger: c.bad,
                ..iced::theme::Palette::DARK
            },
        )
    }

    pub fn view(&self) -> Element<'_, Message> {
        let c = Colors::new(self.appearance);
        let t = self.t();
        let complete = self.selected_document().is_some_and(|document| document.image_count() == 4);
        let bar = row![
            text(TITLE).size(22),
            space().width(16),
            button(t.new_document)
                .style(quiet)
                .on_press_maybe((!self.busy).then_some(Message::NewDocument)),
            button(t.scan_front)
                .style(selected)
                .on_press_maybe(self.can_scan().then_some(Message::Capture(Side::Front))),
            button(t.scan_back)
                .style(quiet)
                .on_press_maybe(self.can_scan().then_some(Message::Capture(Side::Back))),
            button(if self.exporting { t.saving } else { t.pdf })
                .style(quiet)
                .on_press_maybe((complete && !self.exporting).then_some(Message::Export)),
            space().width(Fill),
            self.update_button(),
            button(t.settings).style(quiet).on_press(Message::OpenSettings),
        ]
        .spacing(8)
        .align_y(Alignment::Center);
        let mut body = column![
            container(bar).padding([14, 20]),
            rule::horizontal(1),
            row![
                self.history(),
                rule::vertical(1),
                self.preview(),
                rule::vertical(1),
                self.data_panel()
            ]
            .height(Fill),
            rule::horizontal(1),
            container(
                row![
                    text(self.status.label(t)).size(12).color(c.muted),
                    space().width(Fill),
                    text(if self.backend.is_demo() { t.demo_mode } else { "" })
                        .size(12)
                        .color(c.muted),
                    space().width(16),
                    text(if self.present { t.document_present } else { "" })
                        .size(12)
                        .color(c.muted)
                ]
                .align_y(Alignment::Center)
            )
            .padding([8, 20]),
        ]
        .height(Fill);
        if let Some((message, error)) = &self.notice {
            body = body.push(
                container(
                    row![
                        text(message).size(12).color(if *error { c.bad } else { c.good }),
                        space().width(Fill),
                        button(t.close).style(quiet).on_press(Message::DismissNotice)
                    ]
                    .spacing(12)
                    .align_y(Alignment::Center),
                )
                .padding([10, 20])
                .style(panel),
            );
        }
        let base: Element<'_, Message> = container(body).width(Fill).height(Fill).into();
        if self.settings_open {
            stack![
                base,
                opaque(
                    container(self.settings())
                        .padding(24)
                        .center_x(Fill)
                        .center_y(Fill)
                        .width(Fill)
                        .height(Fill)
                        .style(|_| container::Style {
                            background: Some(Background::Color(Color::from_rgba(0., 0., 0., 0.65))),
                            ..Default::default()
                        })
                )
            ]
            .into()
        } else {
            base
        }
    }

    /// Offered in the top bar only while there is something to do.
    fn update_button(&self) -> Element<'_, Message> {
        let t = self.t();
        match &self.updating {
            Updating::Available(update) => {
                button(text(format!("{} {}", t.update_available, update.version)))
                    .style(selected)
                    .on_press(Message::InstallUpdate)
                    .into()
            }
            Updating::Installing(_) => button(t.installing_update).style(quiet).into(),
            Updating::Installed(_) => {
                button(t.restart).style(selected).on_press(Message::Restart).into()
            }
            _ => space().width(0).into(),
        }
    }

    fn history(&self) -> Element<'_, Message> {
        let c = Colors::new(self.appearance);
        let t = self.t();
        let mut entries = Column::new().spacing(4);
        for (index, document) in self.documents.0.iter().enumerate() {
            let is_selected = self.selected.as_deref() == Some(document.id.as_str());
            let side = if document.front.is_some() { Side::Front } else { Side::Back };
            let mut item = row![].spacing(10).align_y(Alignment::Center);
            if let Some(handle) = self.image_handle(document, side, Source::White) {
                item = item.push(
                    image(handle.clone())
                        .width(54)
                        .height(38)
                        .content_fit(iced::ContentFit::Contain),
                );
            }
            let count =
                usize::from(document.front.is_some()) + usize::from(document.back.is_some());
            let mark = badge::Badge {
                mrz: badge::Mrz::of(document),
                good: c.good,
                bad: c.bad,
                muted: c.muted,
            };
            item = item.push(
                column![
                    text(t.document_title(self.documents.0.len() - index)).size(13),
                    row![
                        text(t.page_count(count)).size(11),
                        iced::widget::canvas(mark).width(badge::SIZE).height(badge::SIZE)
                    ]
                    .spacing(6)
                    .align_y(Alignment::Center)
                ]
                .spacing(4),
            );
            let menu_open = self.history_menu.as_deref() == Some(document.id.as_str());
            entries = entries.push(
                mouse_area(
                    button(item)
                        .width(Fill)
                        .padding(10)
                        .style(if is_selected { selected } else { quiet })
                        .on_press(Message::Select(document.id.clone())),
                )
                .on_right_press(Message::HistoryMenu((!menu_open).then(|| document.id.clone()))),
            );
            if menu_open {
                let complete = document.image_count() == 4;
                entries = entries.push(
                    container(
                        column![
                            button(text(t.save_as).size(12))
                                .width(Fill)
                                .style(quiet)
                                .on_press_maybe(
                                    (complete && !self.exporting)
                                        .then(|| Message::SaveDocumentAs(document.id.clone())),
                                ),
                            button(text(t.delete).size(12).color(c.bad))
                                .width(Fill)
                                .style(quiet)
                                .on_press_maybe(
                                    (!self.busy)
                                        .then(|| Message::DeleteDocument(document.id.clone())),
                                ),
                        ]
                        .spacing(4),
                    )
                    .padding(6)
                    .style(panel),
                );
            }
        }
        if self.documents.0.is_empty() {
            entries = entries.push(text(t.no_documents).size(12).color(c.muted));
        }
        container(
            column![
                row![
                    text(t.documents).size(12).color(c.muted),
                    space().width(Fill),
                    text(self.documents.0.len()).size(12).color(c.muted)
                ],
                scrollable(entries).height(Fill)
            ]
            .spacing(18),
        )
        .width(216)
        .padding(16)
        .height(Fill)
        .into()
    }

    fn preview(&self) -> Element<'_, Message> {
        let c = Colors::new(self.appearance);
        let t = self.t();
        let tabs = row![
            button(t.front)
                .style(if self.preview_side == Side::Front { selected } else { quiet })
                .on_press(Message::PreviewSide(Side::Front)),
            button(t.back)
                .style(if self.preview_side == Side::Back { selected } else { quiet })
                .on_press(Message::PreviewSide(Side::Back)),
            space().width(Fill),
            button(t.white_tab)
                .style(if self.preview_source == Source::White { selected } else { quiet })
                .on_press(Message::PreviewSource(Source::White)),
            button(t.infrared_tab)
                .style(if self.preview_source == Source::Infrared { selected } else { quiet })
                .on_press(Message::PreviewSource(Source::Infrared)),
        ]
        .spacing(6);
        let document = self.selected_document();
        let handle = document.and_then(|document| {
            self.image_handle(document, self.preview_side, self.preview_source)
        });
        let preview: Element<'_, Message> = if let Some(handle) = handle {
            image::Viewer::new(handle.clone())
                .width(Fill)
                .height(Fill)
                .content_fit(iced::ContentFit::Contain)
                .into()
        } else {
            container(
                text(if document.is_none() { t.place_document } else { t.side_missing })
                    .size(18)
                    .color(c.muted),
            )
            .center_x(Fill)
            .center_y(Fill)
            .into()
        };
        let detail = document
            .and_then(|document| document.image(self.preview_side, self.preview_source))
            .map(|image| format!("{} × {}", image.width, image.height))
            .unwrap_or_default();
        container(
            column![
                tabs,
                container(preview).width(Fill).height(Fill).style(move |_| container::Style {
                    background: Some(Background::Color(c.sunk)),
                    ..Default::default()
                }),
                row![
                    text(format!(
                        "{} · {}",
                        t.side_label(self.preview_side),
                        t.source_label(self.preview_source)
                    ))
                    .size(11)
                    .color(c.muted),
                    space().width(Fill),
                    text(detail).size(11).color(c.muted)
                ]
            ]
            .spacing(12),
        )
        .padding(20)
        .width(Fill)
        .height(Fill)
        .into()
    }

    fn data_panel(&self) -> Element<'_, Message> {
        let c = Colors::new(self.appearance);
        let t = self.t();
        let mut data = column![text(t.document_data).size(12).color(c.muted)].spacing(14);
        if let Some(document) = self.selected_document() {
            if let Some(scan) = document.recognised() {
                let (status, color) = match verification(scan) {
                    Verification::Verified => (t.mrz_verified, c.good),
                    Verification::Failed => (t.check_failed, c.bad),
                    Verification::Inconclusive => (t.mrz_unverified, c.muted),
                };
                data = data.push(row![
                    text(status).size(12).color(color),
                    space().width(Fill),
                    text(t.side_label(scan.side)).size(11).color(c.muted)
                ]);
                for field in &scan.fields {
                    data = data.push(
                        column![
                            text(t.field_label(field.kind)).size(11).color(c.muted),
                            text(&field.value).size(14).color(if field.check == Some(false) {
                                c.bad
                            } else {
                                c.ink
                            })
                        ]
                        .spacing(3),
                    );
                }
                if !scan.mrz.is_empty() {
                    data = data.push(rule::horizontal(1)).push(
                        row![
                            text(t.mrz).size(11).color(c.muted),
                            space().width(Fill),
                            button(text(t.copy_mrz).size(11))
                                .style(quiet)
                                .padding([2, 8])
                                .on_press(Message::CopyMrz)
                        ]
                        .align_y(Alignment::Center),
                    );
                    for line in &scan.mrz {
                        data = data.push(text(line).size(10).font(MONO_FONT));
                    }
                }
            } else {
                data = data.push(text(t.no_mrz).size(13).color(c.muted));
            }
            if document.image_count() < 4 {
                data = data
                    .push(rule::horizontal(1))
                    .push(text(t.pdf_image_count(document.image_count())).size(11).color(c.muted));
            }
        } else {
            data = data.push(text("-").color(c.muted));
        }
        container(scrollable(data)).width(318).padding(20).height(Fill).into()
    }

    fn settings(&self) -> Element<'_, Message> {
        let c = Colors::new(self.appearance);
        let t = self.t();
        let label = |value: &'static str| text(value).size(12).color(c.muted);
        let appearance_label = |a: Appearance| match a {
            Appearance::Dark => t.dark,
            Appearance::Light => t.light,
        };
        let appearances =
            Appearance::ALL.map(|value| Choice { value, label: appearance_label(value) });
        let locales = Locale::ALL.map(|value| Choice { value, label: value.native_name() });
        let calibration = self.calibration.as_deref().unwrap_or(t.no_calibration);
        let mut settings = column![
            label(t.language),
            pick_list(
                locales,
                Some(Choice { value: self.locale, label: self.locale.native_name() }),
                |choice| Message::Locale(choice.value),
            )
            .width(Fill),
            label(t.appearance),
            pick_list(
                appearances,
                Some(Choice { value: self.appearance, label: appearance_label(self.appearance) }),
                |choice| Message::Appearance(choice.value),
            )
            .width(Fill),
            rule::horizontal(1),
            toggler(self.auto).label(t.auto_scan).on_toggle(Message::Auto).size(18),
            toggler(self.buzzer).label(t.buzzer).on_toggle(Message::Buzzer).size(18),
            rule::horizontal(1),
            label(t.calibration),
            text(calibration).size(11).color(c.muted),
            text(t.calibration_hint).size(12).color(c.muted),
            row![
                button(t.measure)
                    .style(quiet)
                    .on_press_maybe(self.can_calibrate().then_some(Message::Calibrate)),
                button(t.remove_own).style(quiet).on_press_maybe(
                    (self.can_calibrate() && calibration.contains("PassportScanner"))
                        .then_some(Message::ForgetCalibration)
                )
            ]
            .spacing(8),
        ]
        .spacing(12);
        if self.backend.picks_folders() {
            settings = settings
                .push(rule::horizontal(1))
                .push(label(t.pdf_folder))
                .push(
                    text(self.pdf_folder.as_deref().unwrap_or(t.pdf_folder_ask))
                        .size(12)
                        .color(if self.pdf_folder.is_some() { c.ink } else { c.muted }),
                )
                .push(
                    row![
                        button(t.choose_folder).style(quiet).on_press(Message::ChooseFolder),
                        button(t.reset).style(quiet).on_press_maybe(
                            self.pdf_folder.is_some().then_some(Message::ResetFolder)
                        )
                    ]
                    .spacing(8),
                )
                .push(text(t.pdf_name_hint).size(11).color(c.muted));
        }
        if self.backend.services().is_some() {
            let service = |enabled: bool, name: &'static str, address: &'static str| {
                text(if enabled {
                    format!("{address} · {}", t.active)
                } else {
                    format!("{name} · {}", t.off)
                })
                .size(11)
                .color(c.muted)
            };
            settings = settings
                .push(rule::horizontal(1))
                .push(label(t.server))
                .push(
                    row![
                        toggler(self.web).label(t.web_interface).on_toggle(Message::Web).size(18),
                        space().width(Fill),
                        button(t.copy_address)
                            .style(quiet)
                            .on_press_maybe(self.web.then_some(Message::CopyWebUrl))
                    ]
                    .align_y(Alignment::Center),
                )
                .push(service(self.web, "Web", "localhost:8733"))
                .push(
                    row![
                        toggler(self.mcp).label("MCP").on_toggle(Message::Mcp).size(18),
                        space().width(Fill),
                        button(t.copy_address)
                            .style(quiet)
                            .on_press_maybe(self.mcp.then_some(Message::CopyMcpUrl))
                    ]
                    .align_y(Alignment::Center),
                )
                .push(service(self.mcp, "MCP", "localhost:8735/mcp"));
        }
        settings = settings.push(rule::horizontal(1)).push(
            text(
                self.device
                    .as_ref()
                    .map(|device| device.model.as_str())
                    .unwrap_or(t.scanner_disconnected),
            )
            .size(12)
            .color(c.muted),
        );
        if !self.backend.connected() {
            settings = settings.push(button(t.reconnect).style(quiet).on_press(Message::Reconnect));
        }
        settings = settings
            .push(rule::horizontal(1))
            .push(label(if self.backend.updates() { t.updates } else { t.version }))
            .push(
                text(if self.backend.updates() {
                    format!("{} {VERSION}", t.version)
                } else {
                    VERSION.into()
                })
                .size(12),
            )
            .push(
                row![
                    links::button(links::Link::Website, t.website, c.ink),
                    links::button(links::Link::Repository, "GitHub", c.ink),
                ]
                .spacing(8),
            );
        if self.backend.updates() {
            let (status, color) = match &self.updating {
                Updating::Idle => (String::new(), c.muted),
                Updating::Checking => (t.checking_updates.into(), c.muted),
                Updating::Current => (t.up_to_date.into(), c.muted),
                Updating::Available(update) => {
                    (format!("{}: {}", t.update_available, update.version), c.good)
                }
                Updating::Installing(_) => (t.installing_update.into(), c.muted),
                Updating::Installed(version) => {
                    (format!("{version} · {}", t.update_installed), c.good)
                }
                Updating::Failed(error) => (error.clone(), c.bad),
            };
            if !status.is_empty() {
                settings = settings.push(text(status).size(11).color(color));
            }
            let installs = self.backend.installs_updates();
            let action = match &self.updating {
                Updating::Available(_) if installs => {
                    button(t.install_update).style(selected).on_press(Message::InstallUpdate)
                }
                Updating::Installing(_) => button(t.install_update).style(quiet),
                Updating::Installed(_) => {
                    button(t.restart).style(selected).on_press(Message::Restart)
                }
                Updating::Checking => button(t.check_updates).style(quiet),
                _ => button(t.check_updates).style(quiet).on_press(Message::CheckUpdate),
            };
            settings = settings.push(action).push(
                toggler(self.auto_update)
                    .label(t.auto_update)
                    .on_toggle(Message::AutoUpdate)
                    .size(18),
            );
            if !installs {
                settings = settings.push(text(t.development_build).size(11).color(c.muted));
            }
        }
        if let Some((notice, error)) = &self.notice {
            settings =
                settings.push(text(notice).size(12).color(if *error { c.bad } else { c.good }));
        }
        container(
            column![
                row![
                    text(t.settings).size(20),
                    space().width(Fill),
                    button(t.close).style(quiet).on_press(Message::CloseSettings)
                ]
                .align_y(Alignment::Center),
                rule::horizontal(1),
                scrollable(container(settings).padding(iced::Padding {
                    top: 0.,
                    right: 18.,
                    bottom: 10.,
                    left: 0.,
                }))
                .height(Fill)
            ]
            .spacing(18),
        )
        .padding(24)
        .width(560)
        .height(Fill)
        .max_height(760)
        .style(panel)
        .into()
    }
}

fn panel(theme: &Theme) -> container::Style {
    let c = Colors::theme(theme);
    container::Style {
        background: Some(Background::Color(c.surface)),
        border: Border { color: c.line, width: 1., radius: 2.into() },
        ..Default::default()
    }
}
pub(crate) fn quiet(theme: &Theme, status: button::Status) -> button::Style {
    let c = Colors::theme(theme);
    button::Style {
        background: Some(Background::Color(c.surface)),
        text_color: if status == button::Status::Disabled { c.muted } else { c.ink },
        border: Border {
            color: if status == button::Status::Hovered { c.ink } else { c.line },
            width: 1.,
            radius: 2.into(),
        },
        ..Default::default()
    }
}
fn selected(theme: &Theme, status: button::Status) -> button::Style {
    if status == button::Status::Disabled {
        return quiet(theme, status);
    }
    let c = Colors::theme(theme);
    button::Style {
        background: Some(Background::Color(c.chosen)),
        text_color: c.chosen_ink,
        border: Border { color: c.chosen_line, width: 1., radius: 2.into() },
        ..Default::default()
    }
}
