//! Everything drawn on screen: title bar, sidebar, agent panel, editor
//! panes, bottom bar and overlays. Laid out after Zed's three-column window.

use iced::widget::canvas::Canvas;
use iced::widget::{
    button, column, container, mouse_area, opaque, row, scrollable, space, stack, text,
};
use iced::{Alignment, Color, Element, Length, Padding, Theme};

use crate::editor::EditorView;
use crate::icons::{icon, Icon};
use crate::palette::{self, Palette};
use crate::add_repo::AddRepoMsg;
use crate::chat::ChatMsg;
use crate::collab::CollabMsg;
use crate::panes::Axis;
use crate::scm::ScmMsg;
use crate::terminal_view::TerminalView;
use crate::{menus, theme, App, Handle, Item, Message};

pub(crate) const TITLE_H: f32 = 38.0;
/// Height of the bottom bar and the account chip beside it, so their rules line up.
pub(crate) const BAR_H: f32 = 40.0;
/// Height of every column's header row; a 1px rule runs under each at the same level.
pub(crate) const TAB_H: f32 = 38.0;
pub(crate) const RAIL_W: f32 = 52.0;
pub(crate) const LIST_W: f32 = 232.0;
pub(crate) const AGENT_W: f32 = 470.0;
pub(crate) const SCM_W: f32 = 340.0;
pub(crate) const CHAT_W: f32 = 160.0;
/// Width of a draggable divider (the visible line is 1px in the middle).
pub(crate) const HANDLE_W: f32 = 5.0;

pub(crate) fn divider_h<'a>() -> Element<'a, Message> {
    container(space::vertical().height(Length::Fixed(1.0)))
        .width(Length::Fill)
        .style(theme::bg(theme::BORDER))
        .into()
}

/// A square icon button; `on` highlights it (an open panel, say).
pub(crate) fn icon_button<'a>(glyph: Icon, color: Color, on: bool, msg: Message) -> Element<'a, Message> {
    button(container(icon(glyph, 16.0, if on { theme::TEXT } else { color })).center(Length::Shrink))
        .padding([5, 7])
        .style(theme::flat(on))
        .on_press(msg)
        .into()
}

pub(crate) fn list_row<'a>(content: Element<'a, Message>, selected: bool, msg: Message) -> Element<'a, Message> {
    button(content)
        .width(Length::Fill)
        .padding([5, 8])
        .style(theme::row(selected))
        .on_press(msg)
        .into()
}

/// A list row that starts a drag when pressed (a button only acts on release)
/// and takes the dragged row's place when the pointer enters it.
pub(crate) fn drag_row<'a>(content: Element<'a, Message>, selected: bool, item: Item) -> Element<'a, Message> {
    mouse_area(container(content).width(Length::Fill).padding([5, 8]).style(move |_: &Theme| container::Style {
        background: selected.then(|| theme::SELECTED.into()),
        border: iced::Border { radius: 3.0.into(), ..Default::default() },
        ..Default::default()
    }))
    .on_press(Message::ItemPress(item.clone()))
    .on_enter(Message::ItemEnter(item))
    .interaction(iced::mouse::Interaction::Pointer)
    .into()
}

pub(crate) fn dot<'a>(color: Color) -> Element<'a, Message> {
    container(space::horizontal().width(Length::Fixed(7.0)).height(Length::Fixed(7.0)))
        .style(move |_: &Theme| container::Style {
            background: Some(color.into()),
            border: iced::Border { radius: 3.5.into(), ..Default::default() },
            ..Default::default()
        })
        .into()
}

impl App {
    /// A divider you can drag to resize the panels on either side of it.
    /// `across` is true for a vertical bar between columns.
    pub(crate) fn handle(&self, h: Handle, across: bool) -> Element<'static, Message> {
        let dragging = self.drag.map(|d| d.handle) == Some(h);
        let line = |c: Color| container(space::horizontal().width(Length::Fixed(1.0)).height(Length::Fixed(1.0))).style(theme::bg(c));
        let color = if dragging { theme::ACCENT } else { theme::BORDER };
        if across {
            mouse_area(
                container(line(color).width(Length::Fixed(1.0)).height(Length::Fill))
                    .center_x(Length::Fixed(HANDLE_W))
                    .height(Length::Fill),
            )
            .on_press(Message::DragStart(h))
            .interaction(iced::mouse::Interaction::ResizingHorizontally)
            .into()
        } else {
            mouse_area(
                container(line(color).width(Length::Fill).height(Length::Fixed(1.0)))
                    .center_y(Length::Fixed(HANDLE_W))
                    .width(Length::Fill),
            )
            .on_press(Message::DragStart(h))
            .interaction(iced::mouse::Interaction::ResizingVertically)
            .into()
        }
    }

    pub(crate) fn view(&self) -> Element<'_, Message> {
        let mut main = row![].height(Length::Fill);
        // Source Control wants the width for diffs, so the thread column steps aside.
        let agent_right = self.show_term && self.agent_right;
        let chat_on = self.chat_visible();
        if chat_on && !self.chat_right {
            main = main.push(self.chat_view()).push(self.handle(Handle::Chat, true));
        }
        if self.show_term && !agent_right {
            main = main.push(self.agent_panel()).push(self.handle(Handle::Agent, true));
        }
        main = main.push(self.editor_area());
        if agent_right {
            main = main.push(self.handle(Handle::Agent, true)).push(self.agent_panel());
        }
        if chat_on && self.chat_right {
            main = main.push(self.handle(Handle::Chat, true)).push(self.chat_view());
        }
        let right = column![main, divider_h(), self.bottom_bar()].width(Length::Fill);

        let mut body = row![].height(Length::Fill);
        if self.show_sidebar {
            body = body.push(self.left_block()).push(self.handle(Handle::List, true));
        }
        body = body.push(right);

        let base = column![self.title_bar(), divider_h(), body];        if let Some(p) = &self.palette {
            return stack![
                base,
                mouse_area(space::horizontal().width(Length::Fill).height(Length::Fill))
                    .on_press(Message::ClosePalette),
                self.palette_view(p),
            ]
            .into();
        }
        if self.menu.is_some() {
            return stack![
                base,
                mouse_area(space::horizontal().width(Length::Fill).height(Length::Fill))
                    .on_press(Message::CloseMenu),
                self.menu_dropdown(),
            ]
            .into();
        }
        if self.add_repo.open {
            return stack![base, self.add_repo_dialog()].into();
        }
        if self.collab_ui.open {
            return stack![base, self.collab_panel()].into();
        }
        if self.scm.branch_menu {
            return stack![base, self.branch_menu()].into();
        }
        base.into()
    }

    // --- title bar ---

    fn title_bar(&self) -> Element<'_, Message> {
        let dim = theme::TEXT_DIM;
        let doc_name = self
            .docs
            .get(self.panes.cur())
            .map(|d| d.name())
            .unwrap_or_default();
        container(
            row![
                icon_button(Icon::Menu, dim, self.menu == Some(0), Message::OpenMenu(0)),
                icon_button(Icon::Sidebar, dim, self.show_sidebar, Message::ToggleSidebar),
                icon_button(Icon::Search, dim, false, Message::OpenPalette),
                icon_button(Icon::History, dim, false, Message::Unavailable("History")),
                // The empty stretch of the bar drags the window; double-click maximises.
                mouse_area(
                    container(text(doc_name).size(13).color(theme::TEXT_DIM))
                        .width(Length::Fill)
                        .height(Length::Fill)
                        .center_x(Length::Fill)
                        .align_y(Alignment::Center),
                )
                .on_press(Message::DragWindow)
                .on_double_click(Message::ToggleMaximize),
                icon_button(Icon::Eye, dim, false, Message::Unavailable("Follow mode")),
                self.people_bar(),
                button(
                    row![icon(Icon::Send, 14.0, dim), text("Share").size(13)]
                        .spacing(6)
                        .align_y(Alignment::Center),
                )
                .padding([5, 10])
                .style(theme::flat(false))
                .on_press(Message::Collab(CollabMsg::Panel(true))),
                icon_button(Icon::Minus, dim, false, Message::MinimizeWindow),
                icon_button(Icon::Expand, dim, false, Message::ToggleMaximize),
                icon_button(Icon::Close, dim, false, Message::CloseRequested),
            ]
            .spacing(2)
            .align_y(Alignment::Center)
            .padding([0, 8]),
        )
        .width(Length::Fill)
        .height(Length::Fixed(TITLE_H))
        .align_y(Alignment::Center)
        .style(theme::bg(theme::BAR_BG))
        .into()
    }

    /// The row for one menu entry, shared by the flat and cascading menus.
    fn menu_row(entry: menus::Entry) -> Element<'static, Message> {
        match entry {
            menus::Entry::Separator => container(divider_h()).padding([4, 6]).into(),
            menus::Entry::Header(title) => container(text(title).size(11).color(theme::TEXT_FAINT))
                .padding(Padding { top: 6.0, right: 12.0, bottom: 2.0, left: 12.0 })
                .into(),
            menus::Entry::Item(it) => button(
                row![
                    text(it.label).size(13),
                    space::horizontal(),
                    text(it.shortcut).size(11).color(theme::TEXT_DIM),
                ]
                .spacing(24)
                .align_y(Alignment::Center),
            )
            .width(Length::Fill)
            .padding([5, 12])
            .style(theme::row(false))
            .on_press(Message::MenuPick(Box::new(it.message)))
            .into(),
        }
    }

    fn menu_panel(rows: Vec<Element<'static, Message>>, width: f32) -> Element<'static, Message> {
        container(column(rows).padding(4))
            .width(Length::Fixed(width))
            .style(|_: &Theme| container::Style {
                background: Some(theme::PANEL_BG.into()),
                border: iced::Border { color: theme::SELECTED, width: 1.0, radius: 4.0.into() },
                ..Default::default()
            })
            .into()
    }

    fn menu_dropdown(&self) -> Element<'_, Message> {
        let kind = self.menu.unwrap_or(0);
        let panel: Element<'_, Message> = if kind == 0 || kind == 3 {
            // VS Code style: File / Edit / View / Terminal, each opening a list beside it.
            let top = menus::MENUS.iter().enumerate().map(|(i, title)| {
                mouse_area(
                    button(
                        row![text(*title).size(13), space::horizontal(), text("›").size(13).color(theme::TEXT_DIM)]
                            .align_y(Alignment::Center),
                    )
                    .width(Length::Fill)
                    .padding([5, 12])
                    .style(theme::row(self.submenu == i))
                    .on_press(Message::Submenu(i)),
                )
                .on_enter(Message::Submenu(i))
                .into()
            });
            let top_panel = Self::menu_panel(top.collect(), 150.0);
            let sub_rows = menus::entries(self.submenu, &self.agent_names()).into_iter().map(Self::menu_row);
            let sub_panel = Self::menu_panel(sub_rows.collect(), 280.0);
            // Line the list up with the entry that opened it.
            let offset = if kind == 0 { self.submenu as f32 * 29.0 } else { 0.0 };
            let side = column![space::vertical().height(Length::Fixed(offset)), sub_panel];
            row![top_panel, side].spacing(2).align_y(if kind == 0 { Alignment::Start } else { Alignment::End }).into()
        } else {
            let entries = match kind {
                1 => menus::thread_entries(&self.agent_names()),
                2 => menus::editor_entries(),
                _ => menus::project_entries(),
            };
            container(scrollable(column(entries.into_iter().map(Self::menu_row)).padding(4)).height(Length::Shrink))
                .width(Length::Fixed(300.0))
                .max_height(560.0)
                .style(|_: &Theme| container::Style {
                    background: Some(theme::PANEL_BG.into()),
                    border: iced::Border { color: theme::SELECTED, width: 1.0, radius: 4.0.into() },
                    ..Default::default()
                })
                .into()
        };
        // Each menu opens beside the button that asked for it.
        let sidebar_left = if self.show_sidebar { RAIL_W + self.list_w + HANDLE_W } else { 0.0 };
        let (align_x, align_y, pad) = match kind {
            // Thread column "…": right-aligned under the column header.
            1 => (
                Alignment::Start,
                Alignment::Start,
                Padding { top: TITLE_H + 42.0, left: {
                    let chat = if self.chat_visible() { self.chat_w + HANDLE_W } else { 0.0 };
                    if self.agent_right {
                        (self.win.width - if self.chat_right { chat } else { 0.0 } - 300.0 - 8.0).max(8.0)
                    } else {
                        (sidebar_left + if self.chat_right { 0.0 } else { chat } + self.agent_w - 300.0 - 8.0).max(8.0)
                    }
                }, ..Padding::ZERO },
            ),
            // Editor tabs "…": right edge of the window, under the tab bar.
            2 => (Alignment::End, Alignment::Start, Padding { top: TITLE_H + 42.0, right: 8.0, ..Padding::ZERO }),
            // Account chip: above the chip, bottom-left.
            3 => (Alignment::Start, Alignment::End, Padding { bottom: 56.0, left: 8.0, ..Padding::ZERO }),
            // Repository name: under it, in the file list.
            4 => (Alignment::Start, Alignment::Start, Padding { top: TITLE_H + 42.0, left: RAIL_W + 8.0, ..Padding::ZERO }),
            _ => (Alignment::Start, Alignment::Start, Padding { top: TITLE_H + 4.0, left: 8.0, ..Padding::ZERO }),
        };
        container(opaque(panel))
            .width(Length::Fill)
            .height(Length::Fill)
            .align_x(align_x)
            .align_y(align_y)
            .padding(pad)
            .into()
    }

    fn palette_view(&self, p: &Palette) -> Element<'_, Message> {
        let results = palette::filter(&p.query, &self.agent_names());
        let list: Element<'_, Message> = if results.is_empty() {
            container(text("No matching commands").size(13).color(theme::TEXT_DIM))
                .padding(12)
                .into()
        } else {
            column(results.into_iter().enumerate().map(|(i, cmd)| {
                button(
                    row![
                        text(cmd.label).size(14),
                        space::horizontal(),
                        text(cmd.shortcut).size(12).color(theme::TEXT_DIM),
                    ]
                    .align_y(Alignment::Center),
                )
                .width(Length::Fill)
                .padding([7, 12])
                .style(theme::flat(i == p.selected))
                .on_press(Message::RunPalette(i))
                .into()
            }))
            .padding(4)
            .into()
        };
        let input = container(
            text(if p.query.is_empty() {
                "Type a command…".to_string()
            } else {
                format!("{}▏", p.query)
            })
            .size(15)
            .color(if p.query.is_empty() { theme::TEXT_DIM } else { theme::TEXT }),
        )
        .padding([10, 14])
        .width(Length::Fill);
        let panel = container(column![input, divider_h(), list])
            .width(Length::Fixed(520.0))
            .style(|_: &Theme| container::Style {
                background: Some(theme::PANEL_BG.into()),
                border: iced::Border {
                    color: theme::BORDER,
                    width: 1.0,
                    radius: 4.0.into(),
                },
                ..Default::default()
            });
        container(opaque(panel))
            .width(Length::Fill)
            .height(Length::Fill)
            .align_x(Alignment::Center)
            .padding(iced::Padding::ZERO.top(70.0))
            .into()
    }

    // --- left block: repository rail, channel-style file list, account chip ---

    fn left_block(&self) -> Element<'_, Message> {
        container(column![
            row![self.rail(), self.channel_list()].height(Length::Fill),
            divider_h(),
            self.account_chip(),
        ])
        .width(Length::Fixed(RAIL_W + self.list_w))
        .height(Length::Fill)
        .into()
    }

    /// The far-left strip: one small tile per repository.
    fn rail(&self) -> Element<'_, Message> {
        let mut rail = column![].spacing(6).padding([8, 0]).align_x(Alignment::Start);
        for (i, project) in self.projects.iter().enumerate() {
            let name = self.project_name(i);
            // Marker for a repo you're not looking at that still has something running or unsaved.
            let busy = self.active_project != Some(i)
                && project.ws.as_ref().is_some_and(|w| w.has_running_terminal() || w.has_dirty_docs());
            rail = rail.push(rail_tile(
                RailFace::Text(initials(&name)),
                self.active_project == Some(i),
                busy,
                project.path.display().to_string(),
                Message::ItemPress(Item::Rail(i)),
                Some(Message::ItemEnter(Item::Rail(i))),
            ));
        }
        rail = rail.push(rail_tile(RailFace::Plus, false, false, "Add a repository".into(), Message::AddRepo(AddRepoMsg::Open), None));

        container(scrollable(rail).height(Length::Fill))
            .width(Length::Fixed(RAIL_W))
            .height(Length::Fill)
            .style(theme::bg(theme::RAIL_BG))
            .into()
    }
    /// The repository's files as channels: folders are categories, files are rows.
    fn channel_list(&self) -> Element<'_, Message> {
        let dim = theme::TEXT_DIM;
        let name = self
            .active_project
            .map(|i| self.project_name(i))
            .unwrap_or_else(|| "Hythe".to_string());

        let header = button(
            container(row![text(name).size(15), space::horizontal(), icon(Icon::ChevronDown, 13.0, dim)].align_y(Alignment::Center))
                .height(Length::Fill)
                .align_y(Alignment::Center),
        )
        .width(Length::Fill)
        .height(Length::Fixed(TAB_H))
        .padding([0, 12])
        .style(theme::row(false))
        .on_press(Message::OpenMenu(4));

        let changes = self.scm.changes.len();
        let mut list = column![].spacing(1).padding([6, 8]);
        list = list.push(list_row(
            row![
                icon(Icon::Branch, 15.0, dim),
                text("Changes").size(14).color(dim),
                space::horizontal(),
                text(if changes > 0 { changes.to_string() } else { String::new() }).size(11).color(theme::ACCENT),
            ]
            .spacing(10)
            .align_y(Alignment::Center)
            .into(),
            self.scm.open,
            Message::Scm(ScmMsg::Show),
        ));
        list = list.push(list_row(
            row![icon(Icon::Search, 15.0, dim), text("Search").size(14).color(dim)]
                .spacing(10)
                .align_y(Alignment::Center)
                .into(),
            false,
            Message::OpenPalette,
        ));
        list = list.push(container(divider_h()).padding([6, 4]));

        // Terminals and agents: local to this repository, like voice channels.
        list = list.push(category("Terminals", None, Some(Message::NewTerminal(None))).with_plus());
        for info in self.terms.list() {
            let glyph = if self.agents.iter().any(|a| a.name == info.name) { Icon::Sparkle } else { Icon::Terminal };
            list = list.push(drag_row(
                row![
                    icon(glyph, 15.0, dim),
                    text(info.name.clone()).size(14),
                    space::horizontal(),
                    dot(if info.running { theme::ADDED } else { theme::TEXT_FAINT }),
                ]
                .spacing(10)
                .align_y(Alignment::Center)
                .padding([0, 0])
                .into(),
                self.term_active == Some(info.id),
                Item::Term(info.id),
            ));
        }
        if self.root.is_some() {
            for (i, agent) in self.agents.iter().enumerate().filter(|(_, a)| a.installed) {
                list = list.push(list_row(
                    row![
                        icon(Icon::Sparkle, 15.0, theme::TEXT_FAINT),
                        text(format!("Start {}", agent.name)).size(13).color(theme::TEXT_FAINT),
                    ]
                    .spacing(10)
                    .align_y(Alignment::Center)
                    .into(),
                    false,
                    Message::NewTerminal(Some(i)),
                ));
            }
        }
        list = list.push(container(divider_h()).padding([6, 4]));

        let active_path = self.docs.get(self.panes.cur()).and_then(|d| d.path.as_ref());

        match &self.root {
            None => {
                list = list.push(
                    container(text("No repository open").size(13).color(theme::TEXT_FAINT)).padding([8, 10]),
                );
                list = list.push(list_row(
                    row![icon(Icon::Folder, 15.0, dim), text("Open a repository…").size(14)]
                        .spacing(10)
                        .align_y(Alignment::Center)
                        .into(),
                    false,
                    Message::OpenFolderDialog,
                ));
            }
            Some(root) => {
                // Pinned files in this repository.
                let pinned: Vec<&std::path::PathBuf> =
                    self.persisted.pinned.iter().filter(|p| p.starts_with(&root.path)).collect();
                if !pinned.is_empty() {
                    list = list.push(category("Pinned", None, None));
                    for path in pinned {
                        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                        list = list.push(drag_row(
                            row![icon(Icon::Pin, 14.0, theme::TEXT_FAINT), text(name).size(14)]
                                .spacing(8)
                                .align_y(Alignment::Center)
                                .into(),
                            active_path == Some(path),
                            Item::Pin(path.clone()),
                        ));
                    }
                }

                // Files and folders. A folder is a category; its files are the channels under it.
                let mut rows = Vec::new();
                root.rows(0, &mut rows);
                let mut top_files_started = false;
                for (depth, node) in rows {
                    if node.is_dir {
                        list = list.push(category(
                            &node.name,
                            Some(node.expanded),
                            Some(Message::ToggleDir(node.path.clone())),
                        ).indent(depth));
                    } else {
                        if depth == 0 && !top_files_started {
                            top_files_started = true;
                            list = list.push(category("Files", None, None));
                        }
                        list = list.push(channel_row(
                            Icon::File,
                            node.name.clone(),
                            depth,
                            active_path == Some(&node.path),
                            Message::OpenFile(node.path.clone()),
                        ));
                    }
                }
            }
        }

        container(column![header, divider_h(), scrollable(list).height(Length::Fill)])
            .width(Length::Fixed(self.list_w))
            .height(Length::Fill)
            .style(theme::bg(theme::PANEL_BG))
            .into()
    }

    /// Account panel across the bottom of the rail and list, like Discord's.
    fn account_chip(&self) -> Element<'_, Message> {
        let dim = theme::TEXT_DIM;
        let initial = self.user.chars().next().map(|c| c.to_ascii_uppercase().to_string()).unwrap_or_default();
        let avatar: Element<'_, Message> = match &self.avatar {
            Some(handle) => iced::widget::image(handle.clone())
                .width(Length::Fixed(28.0))
                .height(Length::Fixed(28.0))
                .border_radius(14.0)
                .into(),
            None => container(text(initial).size(13).color(Color::WHITE))
                .center(Length::Fixed(28.0))
                .style(|_: &Theme| container::Style {
                    background: Some(Color::from_rgb8(0x5a, 0x7f, 0xb8).into()),
                    border: iced::Border { radius: 14.0.into(), ..Default::default() },
                    ..Default::default()
                })
                .into(),
        };
        let subtitle = self
            .git
            .as_ref()
            .map(|g| g.branch.clone())
            .unwrap_or_else(|| "no repository".into());
        container(
            row![
                avatar,
                column![text(self.user.clone()).size(13), text(subtitle).size(11).color(dim)].spacing(1),
                space::horizontal(),
                icon_button(Icon::Terminal, dim, self.show_term, Message::ToggleTerminal),
                icon_button(Icon::Menu, dim, self.menu == Some(3), Message::OpenMenu(3)),
            ]
            .spacing(10)
            .align_y(Alignment::Center)
            .padding([0, 10]),
        )
        .width(Length::Fill)
        .height(Length::Fixed(BAR_H))
        .align_y(Alignment::Center)
        .style(theme::bg(theme::CHIP_BG))
        .into()
    }
    // --- agent / terminal panel ---

    fn agent_panel(&self) -> Element<'_, Message> {
        let session = self.term_active.and_then(|id| self.terms.get(id));
        let title = session.as_ref().map(|s| s.name().to_string()).unwrap_or_else(|| "New Thread".into());
        let dim = theme::TEXT_DIM;

        let header = container(
            row![
                text(title).size(14),
                space::horizontal(),
                icon_button(Icon::Plus, dim, false, Message::NewTerminal(None)),
                icon_button(Icon::Expand, dim, false, Message::Unavailable("Zoom")),
                icon_button(Icon::More, dim, self.menu == Some(1), Message::OpenMenu(1)),
            ]
            .spacing(2)
            .align_y(Alignment::Center)
            .padding([0, 10]),
        )
        .width(Length::Fill)
        .height(Length::Fixed(TAB_H))
        .align_y(Alignment::Center);
        // Grab the header and drag it across the window to dock the column on the other side.
        let header = mouse_area(header)
            .on_press(Message::DragStart(Handle::Dock))
            .interaction(iced::mouse::Interaction::Grab);

        let body: Element<'_, Message> = match (&self.term_snap, session) {
            (Some(snap), Some(session)) => Canvas::new(TerminalView { snap, session, focused: self.focus_term })
                .width(Length::Fill)
                .height(Length::Fill)
                .into(),
            _ => {
                let mut starters = column![
                    text("Start a thread").size(15),
                    text("Run a shell or a coding agent in this folder.").size(12).color(theme::TEXT_DIM),
                    space::vertical().height(Length::Fixed(10.0)),
                    list_row(
                        row![icon(Icon::Terminal, 15.0, theme::TEXT_DIM), text("New Terminal").size(13)]
                            .spacing(10)
                            .align_y(Alignment::Center)
                            .into(),
                        false,
                        Message::NewTerminal(None),
                    ),
                ]
                .spacing(4)
                .width(Length::Fixed(280.0));
                // Agents live in a dropdown: installed ones first, the rest dimmed.
                let installed = self.agents.iter().filter(|a| a.installed).count();
                let open = self.agent_menu;
                starters = starters.push(
                    button(
                        row![
                            icon(Icon::Sparkle, 15.0, theme::ACCENT),
                            text("Start an agent").size(13),
                            space::horizontal(),
                            text(format!("{installed} installed")).size(11).color(theme::TEXT_FAINT),
                            icon(if open { Icon::ChevronDown } else { Icon::ChevronRight }, 11.0, theme::TEXT_DIM),
                        ]
                        .spacing(10)
                        .align_y(Alignment::Center),
                    )
                    .width(Length::Fill)
                    .padding([7, 10])
                    .style(move |_: &Theme, status| button::Style {
                        background: Some(theme::EDITOR_BG.into()),
                        text_color: theme::TEXT,
                        border: iced::Border {
                            color: if open || matches!(status, button::Status::Hovered) { theme::ACCENT } else { theme::SELECTED },
                            width: 1.0,
                            radius: 3.0.into(),
                        },
                        ..button::Style::default()
                    })
                    .on_press(Message::AgentMenu(!open)),
                );
                if open {
                    let mut order: Vec<usize> = (0..self.agents.len()).collect();
                    order.sort_by_key(|&i| !self.agents[i].installed);
                    let mut items = column![].spacing(1).padding(3);
                    for i in order {
                        let agent = &self.agents[i];
                        items = items.push(list_row(
                            row![
                                icon(Icon::Sparkle, 14.0, if agent.installed { theme::ACCENT } else { theme::TEXT_FAINT }),
                                text(agent.name.clone())
                                    .size(13)
                                    .color(if agent.installed { theme::TEXT } else { theme::TEXT_FAINT }),
                                space::horizontal(),
                                text(if agent.installed { "" } else { "not installed" }).size(11).color(theme::TEXT_FAINT),
                            ]
                            .spacing(10)
                            .align_y(Alignment::Center)
                            .into(),
                            false,
                            Message::NewTerminal(Some(i)),
                        ));
                    }
                    starters = starters.push(container(items).style(|_: &Theme| container::Style {
                        background: Some(theme::EDITOR_BG.into()),
                        border: iced::Border { color: theme::SELECTED, width: 1.0, radius: 3.0.into() },
                        ..Default::default()
                    }));
                }                container(starters).center_x(Length::Fill).padding(iced::Padding { top: 120.0, ..iced::Padding::ZERO }).into()
            }
        };

        // The rule under the header lights up while the keyboard is in this column.
        let rule = container(space::vertical().height(Length::Fixed(1.0)))
            .width(Length::Fill)
            .style(theme::bg(if self.focus_term { theme::ACCENT } else { theme::BORDER }));
        container(column![header, rule, body])
            .width(Length::Fixed(self.agent_w))
            .height(Length::Fill)
            .style(theme::bg(theme::AGENT_BG))
            .into()
    }

    // --- editor area ---

    fn editor_area(&self) -> Element<'_, Message> {
        if self.scm.open {
            // Source Control is one more tab: the file tabs stay in view above it.
            return container(column![self.tab_bar(0), self.scm_view()])
                .width(Length::Fill)
                .height(Length::Fill)
                .style(theme::bg(theme::EDITOR_BG))
                .into();
        }
        if self.panes.list.len() == 1 && self.panes.list[0].tabs.is_empty() {
            return container(self.welcome())
                .width(Length::Fill)
                .height(Length::Fill)
                .style(theme::bg(theme::EDITOR_BG))
                .into();
        }
        let mut views = Vec::new();
        for i in 0..self.panes.list.len() {
            if i > 0 {
                views.push(self.handle(Handle::Panes(i - 1), self.panes.axis == Axis::Horizontal));
            }
            views.push(self.pane_view(i));
        }
        match self.panes.axis {
            Axis::Horizontal => row(views).width(Length::Fill).height(Length::Fill).into(),
            Axis::Vertical => column(views).width(Length::Fill).height(Length::Fill).into(),
        }
    }

    fn pane_view(&self, index: usize) -> Element<'_, Message> {
        let pane = &self.panes.list[index];
        let focused = index == self.panes.focus;
        let current = pane.current().and_then(|t| self.docs.get(t.doc).map(|d| (t, d)));
        let body: Element<'_, Message> = match current {
            Some((tab, doc)) => Canvas::new(EditorView {
                doc,
                scroll: tab.scroll,
                pane: index,
                focused,
                viewport: &pane.viewport,
                popup: if focused && !self.focus_term { self.completion.as_ref() } else { None },
                blame: if focused { self.blame_text(doc) } else { None },
                remotes: self.remote_cursors(tab.doc),
            })
            .width(Length::Fill)
            .height(Length::Fill)
            .into(),
            None => container(text("Open a file here").size(13).color(theme::TEXT_DIM))
                .center(Length::Fill)
                .into(),
        };
        let breadcrumb = current.map(|(_, doc)| self.breadcrumb(doc.path.as_deref(), doc.name()));
        let mut col = column![self.tab_bar(index)];
        if let Some(b) = breadcrumb {
            col = col.push(b);
        }
        let share = Length::FillPortion((pane.weight * 1000.0).round().clamp(1.0, 60000.0) as u16);
        let (w, h) = match self.panes.axis {
            Axis::Horizontal => (share, Length::Fill),
            Axis::Vertical => (Length::Fill, share),
        };
        container(col.push(body))
            .width(w)
            .height(h)
            .style(theme::bg(theme::EDITOR_BG))
            .into()
    }

    /// Path of the file below the project root, with a search button at the end.
    fn breadcrumb(&self, path: Option<&std::path::Path>, name: String) -> Element<'_, Message> {
        let label = match (path, &self.root) {
            (Some(p), Some(root)) => p
                .strip_prefix(&root.path)
                .map(|rel| rel.to_string_lossy().replace('\\', "/"))
                .unwrap_or_else(|_| p.to_string_lossy().replace('\\', "/")),
            _ => name,
        };
        container(
            row![
                text(label).size(13).color(theme::TEXT_DIM),
                space::horizontal(),
                icon_button(Icon::Search, theme::TEXT_FAINT, false, Message::OpenPalette),
            ]
            .align_y(Alignment::Center)
            .padding(Padding { top: 0.0, right: 8.0, bottom: 0.0, left: 16.0 }),
        )
        .width(Length::Fill)
        .height(Length::Fixed(30.0))
        .align_y(Alignment::Center)
        .into()
    }

    fn tab_bar(&self, pane_index: usize) -> Element<'_, Message> {
        let pane = &self.panes.list[pane_index];
        let tabs = pane.tabs.iter().enumerate().filter_map(|(i, t)| {
            let d = self.docs.get(t.doc)?;
            // No file tab is "current" while the Changes tab is.
            let active = i == pane.active && !(self.scm.open && pane_index == 0);
            let end: Element<'_, Message> = if d.dirty {
                button(container(dot(theme::TEXT_DIM)).center(Length::Shrink))
                    .padding([8, 8])
                    .style(theme::flat(false))
                    .on_press(Message::CloseTab { pane: pane_index, tab: i })
                    .into()
            } else {
                icon_button(Icon::Close, theme::TEXT_FAINT, false, Message::CloseTab { pane: pane_index, tab: i })
            };
            // Pressing selects the tab and lets go of it again on release; dragging it
            // over another tab (in this pane or another) moves it there.
            let tab = row![
                mouse_area(
                    container(text(d.name()).size(13).color(if active { theme::TEXT } else { theme::TEXT_DIM }))
                        .padding([8, 6]),
                )
                .on_press(Message::TabPress { pane: pane_index, tab: i })
                .interaction(iced::mouse::Interaction::Pointer),
                end,
            ]
            .align_y(Alignment::Center)
            .padding([0, 4]);
            Some(
                mouse_area(
                    container(tab)
                        .height(Length::Fixed(TAB_H))
                        .align_y(Alignment::Center)
                        .style(theme::bg(if active { theme::EDITOR_BG } else { theme::TAB_BAR_BG })),
                )
                .on_enter(Message::TabEnter { pane: pane_index, tab: i })
                .into(),
            )
        });
        let tabs = scrollable(row(tabs)).direction(scrollable::Direction::Horizontal(
            scrollable::Scrollbar::new().width(0).scroller_width(0),
        ));
        let bar = container(
            row![
                tabs,
                space::horizontal(),
                icon_button(Icon::Split, theme::TEXT_DIM, self.panes.list.len() > 1, Message::Split(Axis::Horizontal)),
                icon_button(Icon::More, theme::TEXT_DIM, self.menu == Some(2), Message::OpenMenu(2)),
            ]
            .align_y(Alignment::Center)
            .padding([0, 6]),
        )
        .width(Length::Fill)
        .height(Length::Fixed(TAB_H))
        .style(theme::bg(theme::TAB_BAR_BG));
        column![bar, divider_h()].into()
    }

    fn welcome(&self) -> Element<'_, Message> {
        let action = |glyph: Icon, label: &'static str, hint: &'static str, msg: Message| {
            button(
                row![
                    icon(glyph, 15.0, theme::TEXT_DIM),
                    text(label).size(14),
                    space::horizontal(),
                    text(hint).size(12).color(theme::TEXT_DIM),
                ]
                .spacing(10)
                .align_y(Alignment::Center),
            )
            .width(Length::Fixed(360.0))
            .padding([8, 12])
            .style(theme::row(false))
            .on_press(msg)
        };
        container(
            column![
                text("Hythe").size(34).color(theme::ACCENT),
                text("a fast editor, built in Rust").size(14).color(theme::TEXT_DIM),
                space::vertical().height(Length::Fixed(24.0)),
                action(Icon::File, "New File", "Ctrl+N", Message::NewFile),
                action(Icon::File, "Open File", "Ctrl+O", Message::OpenFileDialog),
                action(Icon::Folder, "Open Folder", "Ctrl+Shift+O", Message::OpenFolderDialog),
                action(Icon::Search, "Command Palette", "Ctrl+Shift+P", Message::OpenPalette),
                action(Icon::Terminal, "Toggle Terminal", "Ctrl+`", Message::ToggleTerminal),
            ]
            .spacing(4)
            .align_x(Alignment::Center),
        )
        .center(Length::Fill)
        .into()
    }

    // --- bottom bar ---

    /// A one-line box in the bottom bar: Enter sends the text to the repository chat.
    fn message_bar(&self) -> Element<'_, Message> {
        // While the chat is on screen it has its own message box.
        if self.repo.is_none() || self.chat_visible() {
            return space::horizontal().width(Length::Shrink).into();
        }
        iced::widget::text_input("Message the team…", &self.chat.input)
            .on_input(|s| Message::Chat(ChatMsg::Input(s)))
            .on_submit(Message::Chat(ChatMsg::Send))
            .padding([3, 8])
            .size(12)
            .width(Length::Fixed(260.0))
            .style(theme::input)
            .into()
    }

    fn bottom_bar(&self) -> Element<'_, Message> {
        let dim = theme::TEXT_DIM;

        // Repository, branch and uncommitted changes.
        let git: Element<'_, Message> = match &self.git {
            Some(g) => button(
                row![
                    icon(Icon::Folder, 13.0, dim),
                    text(g.repo.clone()).size(12),
                    text("/").size(12).color(theme::TEXT_FAINT),
                    text(g.branch.clone()).size(12),
                    text(format!("+{}", g.added)).size(12).color(theme::ADDED),
                    text(format!("−{}", g.removed)).size(12).color(theme::REMOVED),
                ]
                .spacing(6)
                .align_y(Alignment::Center),
            )
            .padding([3, 8])
            .style(theme::flat(false))
            .on_press(Message::Scm(ScmMsg::BranchMenu(true)))
            .into(),
            None => text("No repository").size(12).color(theme::TEXT_FAINT).into(),
        };

        let position = self
            .docs
            .get(self.panes.cur())
            .map(|d| {
                let (l, c) = d.buffer.cursor();
                format!("Ln {}, Col {}   {}", l + 1, c + 1, d.language_name())
            })
            .unwrap_or_default();

        let session_state: Element<'_, Message> = match self.term_active.and_then(|id| self.terms.get(id)) {
            Some(s) => row![
                dot(if s.is_running() { theme::ACCENT } else { theme::TEXT_FAINT }),
                text(if s.is_running() { "Running" } else { "Exited" }).size(12).color(dim),
                text(s.name().to_string()).size(12).color(dim),
            ]
            .spacing(6)
            .align_y(Alignment::Center)
            .into(),
            None => space::horizontal().width(Length::Shrink).into(),
        };

        let icons = row![
            icon_button(Icon::Terminal, dim, self.show_term, Message::ToggleTerminal),
            icon_button(Icon::Bug, dim, false, Message::Unavailable("The debugger")),
            icon_button(Icon::Outline, dim, false, Message::Unavailable("Outline")),
            icon_button(Icon::Chat, dim, self.chat_visible(), Message::Chat(if self.chat_visible() { ChatMsg::Hide } else { ChatMsg::Show })),
            icon_button(Icon::Branch, dim, self.scm.open, Message::Scm(ScmMsg::Show)),
            icon_button(Icon::Panels, dim, self.show_sidebar, Message::ToggleSidebar),
        ]
        .spacing(0);

        container(
            row![
                git,
                self.message_bar(),
                container(text(self.status.clone()).size(12).color(theme::TEXT_FAINT).wrapping(text::Wrapping::None))
                    .width(Length::Fill)
                    .clip(true)
                    .padding([0, 10]),
                session_state,
                text(position).size(12).color(dim),
                icons,
            ]
            .spacing(12)
            .align_y(Alignment::Center),
        )
        .width(Length::Fill)
        .height(Length::Fixed(BAR_H))
        .align_y(Alignment::Center)
        .style(theme::bg(theme::BAR_BG))
        .into()
    }
}

impl App {
    /// "You, 2 hours ago • subject" for the cursor line, when blame is loaded
    /// and the file hasn't been edited since (line numbers would drift).
    pub(crate) fn blame_text(&self, doc: &crate::doc::Doc) -> Option<String> {
        if doc.dirty {
            return None;
        }
        let (line, _) = doc.buffer.cursor();
        let b = doc.blame.as_ref()?.get(line)?;
        if b.uncommitted {
            return Some("You • Uncommitted changes".into());
        }
        let who = if b.author == self.user { "You" } else { b.author.as_str() };
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        Some(format!("{who}, {} • {}", hythe_git::time_ago(b.time, now), b.summary))
    }
}
impl App {
    /// Everyone working on this repository as small round pictures in the title
    /// bar; hovering one shows the name. Shown only when there is more than one.
    pub(crate) fn people_bar(&self) -> Element<'_, Message> {
        if !self.show_members() {
            return space::horizontal().width(Length::Shrink).into();
        }
        const SHOWN: usize = 7;
        const SIZE: f32 = 22.0;
        let face = |name: &str, handle: Option<&iced::widget::image::Handle>, color: Color, ring: Option<Color>| -> Element<'static, Message> {
            let border = iced::Border { radius: (SIZE / 2.0).into(), color: ring.unwrap_or(Color::TRANSPARENT), width: if ring.is_some() { 2.0 } else { 0.0 } };
            match handle {
                Some(h) => container(iced::widget::image(h.clone()).width(Length::Fixed(SIZE)).height(Length::Fixed(SIZE)).border_radius(SIZE / 2.0))
                    .style(move |_: &Theme| container::Style { border, ..Default::default() })
                    .into(),
                None => {
                    let initial = name.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default();
                    container(text(initial).size(11).color(Color::WHITE))
                        .center(Length::Fixed(SIZE))
                        .style(move |_: &Theme| container::Style { background: Some(color.into()), border, ..Default::default() })
                        .into()
                }
            }
        };
        let tip = |face: Element<'static, Message>, label: String| -> Element<'static, Message> {
            iced::widget::tooltip(
                face,
                container(text(label).size(12)).padding([3, 7]).style(|_: &Theme| container::Style {
                    background: Some(theme::BAR_BG.into()),
                    border: iced::Border { color: theme::SELECTED, width: 1.0, radius: 2.0.into() },
                    ..Default::default()
                }),
                iced::widget::tooltip::Position::Bottom,
            )
            .gap(6)
            .into()
        };

        let mut r = row![].spacing(4).align_y(Alignment::Center);
        // People in a live session first, ringed in green.
        if let Some(c) = &self.collab {
            for p in &c.peers {
                let color = crate::collab::rgb(p.color);
                r = r.push(tip(face(&p.name, None, color, Some(theme::ADDED)), format!("{} (online)", p.name)));
            }
        }
        for m in self.members.iter().take(SHOWN) {
            let hue = m.name.bytes().fold(0u32, |a, b| a.wrapping_mul(31).wrapping_add(b as u32)) % 360;
            let handle = m.login.as_ref().and_then(|l| self.member_avatars.get(l));
            let label = format!("{} · {} commit{}", m.name, m.commits, if m.commits == 1 { "" } else { "s" });
            r = r.push(tip(face(&m.name, handle, hsv(hue as f32), None), label));
        }
        if self.members.len() > SHOWN {
            let rest: Vec<&str> = self.members[SHOWN..].iter().map(|m| m.name.as_str()).collect();
            let more = container(text(format!("+{}", rest.len())).size(10).color(theme::TEXT_DIM))
                .center(Length::Fixed(SIZE))
                .style(|_: &Theme| container::Style {
                    border: iced::Border { color: theme::SELECTED, width: 1.0, radius: (SIZE / 2.0).into() },
                    ..Default::default()
                });
            r = r.push(tip(more.into(), rest.join(", ")));
        }
        container(r).padding([0, 6]).into()
    }
}

/// A muted colour for a hue in degrees.
pub(crate) fn hsv(h: f32) -> Color {
    let (s, v) = (0.45f32, 0.62f32);
    let c = v * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = v - c;
    let (r, g, b) = match (h / 60.0) as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    Color::from_rgb(r + m, g + m, b + m)
}

/// Letters for a repository tile: the first two of its name, "Hythe-Prototype" becomes "Hy".
pub(crate) fn initials(name: &str) -> String {
    let mut letters = name.chars().filter(|c| c.is_alphanumeric()).take(2);
    let first: String = letters.next().into_iter().flat_map(char::to_uppercase).collect();
    let rest: String = letters.flat_map(char::to_lowercase).collect();
    if first.is_empty() { "?".into() } else { first + &rest }
}

enum RailFace {
    Text(String),
    Plus,
}

/// One tile in the rail: a small square, brighter when it's the open repository,
/// with a thin marker on its left edge when open (tall) or busy (short).
fn rail_tile<'a>(face: RailFace, active: bool, busy: bool, tip: String, msg: Message, enter: Option<Message>) -> Element<'a, Message> {
    let bg = if active { theme::TILE_ACTIVE } else { theme::TILE_BG };
    let fg = if active { Color::WHITE } else { theme::TEXT_DIM };
    let inner: Element<'a, Message> = match face {
        RailFace::Text(label) => text(label).size(12).color(fg).center().into(),
        RailFace::Plus => icon(Icon::Plus, 14.0, theme::TEXT_DIM),
    };
    let tile = container(inner)
        .center(Length::Fixed(34.0))
        .style(move |_: &Theme| container::Style {
            background: Some(bg.into()),
            border: iced::Border { radius: 4.0.into(), ..Default::default() },
            ..Default::default()
        });
    let marker_height = if active { 18.0 } else if busy { 6.0 } else { 0.0 };
    let marker = container(space::vertical().height(Length::Fixed(marker_height)).width(Length::Fixed(3.0)))
        .style(|_: &Theme| container::Style { background: Some(theme::ACCENT.into()), ..Default::default() });
    let button = mouse_area(tile).on_press(msg).interaction(iced::mouse::Interaction::Pointer);
    let content = container(row![marker, space::horizontal().width(Length::Fixed(6.0)), button].align_y(Alignment::Center))
        .width(Length::Fixed(RAIL_W))
        .height(Length::Fixed(36.0))
        .align_y(Alignment::Center);
    let content: Element<'a, Message> = match enter {
        Some(m) => mouse_area(content).on_enter(m).into(),
        None => content.into(),
    };
    iced::widget::tooltip(
        content,
        container(text(tip).size(12)).padding([3, 7]).style(|_: &Theme| container::Style {
            background: Some(theme::BAR_BG.into()),
            border: iced::Border { color: theme::SELECTED, width: 1.0, radius: 2.0.into() },
            ..Default::default()
        }),
        iced::widget::tooltip::Position::Right,
    )
    .gap(4)
    .into()
}
/// A category header: small caps title with an open/closed chevron. `indent`
/// nests sub-folders; `with_plus` adds a "+" button on the right.
pub(crate) struct Category<'a> {
    title: String,
    expanded: Option<bool>,
    on_press: Option<Message>,
    indent: usize,
    plus: bool,
    _marker: std::marker::PhantomData<&'a ()>,
}

fn category<'a>(title: &str, expanded: Option<bool>, on_press: Option<Message>) -> Category<'a> {
    Category { title: title.to_string(), expanded, on_press, indent: 0, plus: false, _marker: std::marker::PhantomData }
}

impl<'a> Category<'a> {
    fn indent(mut self, depth: usize) -> Self {
        self.indent = depth;
        self
    }

    fn with_plus(mut self) -> Self {
        self.plus = true;
        self
    }
}

impl<'a> From<Category<'a>> for Element<'a, Message> {
    fn from(c: Category<'a>) -> Self {
        let dim = theme::TEXT_DIM;
        let chevron: Element<'a, Message> = match c.expanded {
            Some(true) => icon(Icon::ChevronDown, 10.0, dim),
            Some(false) => icon(Icon::ChevronRight, 10.0, dim),
            None => space::horizontal().width(Length::Fixed(0.0)).into(),
        };
        let mut r = row![
            space::horizontal().width(Length::Fixed(c.indent as f32 * 10.0)),
            chevron,
            text(c.title.to_uppercase()).size(11).color(dim),
        ]
        .spacing(5)
        .align_y(Alignment::Center);
        if c.plus {
            r = r.push(space::horizontal());
            r = r.push(icon(Icon::Plus, 13.0, dim));
        }
        let b = button(r).width(Length::Fill).padding(Padding { top: 10.0, right: 6.0, bottom: 3.0, left: 6.0 }).style(|_: &Theme, status| {
            button::Style {
                text_color: if matches!(status, button::Status::Hovered) { theme::TEXT } else { theme::TEXT_DIM },
                ..button::Style::default()
            }
        });
        match c.on_press {
            Some(msg) => b.on_press(msg).into(),
            None => b.into(),
        }
    }
}

/// A file as a channel row.
fn channel_row<'a>(glyph: Icon, name: String, depth: usize, selected: bool, msg: Message) -> Element<'a, Message> {
    let color = if selected { theme::TEXT } else { theme::TEXT_DIM };
    list_row(
        row![
            space::horizontal().width(Length::Fixed(depth as f32 * 10.0 + 4.0)),
            icon(glyph, 14.0, theme::TEXT_FAINT),
            text(name).size(14).color(color),
        ]
        .spacing(8)
        .align_y(Alignment::Center)
        .into(),
        selected,
        msg,
    )
}