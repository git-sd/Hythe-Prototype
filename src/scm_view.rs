//! Drawing for source control: the Changes view and the branch picker.

use hythe_git::{time_ago, Change, DiffLine, LineKind, Status};
use iced::widget::{button, column, container, mouse_area, opaque, row, scrollable, space, stack, text, text_input};
use iced::{Alignment, Color, Element, Font, Length, Padding, Theme};

use crate::icons::{icon, Icon};
use crate::scm::{ScmMsg, ScmPlace, ScmTab};
use crate::view::{divider_h, icon_button, list_row, BAR_H};
use crate::Handle;
use crate::{theme, App, Message};

const MAX_DIFF_LINES: usize = 3000;

fn scm(msg: ScmMsg) -> Message {
    Message::Scm(msg)
}

fn status_color(s: Status) -> Color {
    match s {
        Status::Added | Status::Untracked => theme::ADDED,
        Status::Deleted | Status::Conflicted => theme::REMOVED,
        _ => theme::ACCENT,
    }
}

impl App {
    /// Roughly how wide the diff is: what the window leaves after the sidebars.
    fn diff_pane_w(&self) -> f32 {
        use crate::view::{HANDLE_W, RAIL_W};
        let sidebar = if self.show_sidebar { RAIL_W + self.list_w + HANDLE_W } else { 0.0 };
        let agent = if self.show_term && self.chat_window.is_none() { self.agent_w + HANDLE_W } else { 0.0 };
        let chat = if self.chat_docked() { self.chat_w + HANDLE_W } else { 0.0 };
        if self.scm_window.is_some() {
            (self.win.width * 0.7 - self.scm_w).max(300.0)
        } else {
            let _ = (sidebar, agent, chat);
        (self.scm_detail_w - 20.0).max(240.0)
        }
    }

    /// History is open as a column on the left (not popped out into its own window).
    pub(crate) fn scm_docked(&self) -> bool {
        self.scm.open && self.scm_window.is_none()
    }

    /// Width the docked History column takes, handle included.
    pub(crate) fn scm_l(&self) -> f32 {
        let history = if self.scm_docked() { self.scm_w + crate::view::HANDLE_W } else { 0.0 };
        let detail = if self.scm_showing_diff() { self.scm_detail_w + crate::view::HANDLE_W } else { 0.0 };
        history + detail
    }

    /// The details column beside History: the chosen file's diff, or the chosen commit
    /// with its files and diff. It opens when you click something in History.
    pub(crate) fn scm_detail_panel(&self) -> Element<'_, Message> {
        let commit = self.scm.tab == ScmTab::History;
        let title = if commit {
            self.scm
                .selected_commit
                .as_ref()
                .and_then(|sha| self.scm.commits.iter().find(|c| &c.sha == sha))
                .map(|c| format!("Commit {}", c.short))
                .unwrap_or_else(|| "Commit".into())
        } else {
            self.scm.selected.as_ref().map(|(p, _)| p.clone()).unwrap_or_default()
        };
        let head = container(
            row![
                text(title).size(13).wrapping(text::Wrapping::None),
                space::horizontal(),
                icon_button(Icon::Close, theme::TEXT_FAINT, false, scm(ScmMsg::CloseDiff)),
            ]
            .push(self.header_extra(crate::view::Edge::ScmDetail))
            .spacing(2)
            .align_y(Alignment::Center)
            .padding([0, 8]),
        )
        .width(Length::Fill)
        .height(Length::Fixed(crate::view::TAB_H))
        .align_y(Alignment::Center);
        container(column![head, divider_h(), self.scm_diff()])
            .width(Length::Fixed(self.scm_detail_w))
            .height(Length::Fill)
            .style(theme::bg(theme::EDITOR_BG))
            .into()
    }

    /// A changed file or a commit's file is chosen, so its diff should show beside the code.
    pub(crate) fn scm_showing_diff(&self) -> bool {
        self.scm_docked() && self.scm.show_diff
    }

    /// History as a column on the left, like the sidebar: Changes and commits in one list.
    /// Choosing a file or a commit's file shows its diff in the editor area.
    pub(crate) fn scm_sidebar(&self) -> Element<'_, Message> {
        let head = container(
            row![
                text("History").size(13),
                space::horizontal(),
                icon_button(Icon::Expand, theme::TEXT_FAINT, false, scm(ScmMsg::PopOut)),
                icon_button(Icon::Close, theme::TEXT_FAINT, false, scm(ScmMsg::Hide)),
            ]
            .push(self.header_extra(crate::view::Edge::Scm))
            .spacing(2)
            .align_y(Alignment::Center)
            .padding([0, 8]),
        )
        .width(Length::Fill)
        .height(Length::Fixed(crate::view::TAB_H))
        .align_y(Alignment::Center);
        container(column![head, divider_h(), self.scm_left()])
            .width(Length::Fixed(self.scm_w))
            .height(Length::Fill)
            .style(theme::bg(theme::PANEL_BG))
            .into()
    }

    /// Source Control in a window of its own: the list on the left, the diff on the right.
    pub(crate) fn scm_view_in(&self, _place: ScmPlace) -> Element<'_, Message> {
        let left_head = column![
            container(self.mode_switch())
                .width(Length::Fixed(self.scm_w))
                .height(Length::Fixed(crate::view::TAB_H))
                .padding([0, 10])
                .align_y(Alignment::Center)
                .style(theme::bg(theme::PANEL_BG)),
            divider_h(),
        ];
        // A fixed-height stand-in for the drag handle below, so the header row stays one row tall.
        let gap = container(
            container(space::horizontal().width(Length::Fixed(1.0)))
                .height(Length::Fill)
                .style(theme::bg(theme::BORDER)),
        )
        .center_x(Length::Fixed(crate::view::HANDLE_W))
        .height(Length::Fixed(crate::view::TAB_H + 1.0));
        let right_head = column![
            container(
                row![
                    space::horizontal(),
                    icon_button(Icon::Panels, theme::TEXT_FAINT, false, scm(ScmMsg::PopIn)),
                    icon_button(Icon::Close, theme::TEXT_FAINT, false, scm(ScmMsg::Hide)),
                ]
                .spacing(2)
                .align_y(Alignment::Center)
                .padding([0, 8]),
            )
            .width(Length::Fill)
            .height(Length::Fixed(crate::view::TAB_H))
            .style(theme::bg(theme::TAB_BAR_BG)),
            divider_h(),
        ];
        let head = row![left_head, gap, right_head];
        let body = row![self.scm_left(), self.handle(Handle::Scm, true), self.scm_diff()].height(Length::Fill);
        container(column![head, body])
            .width(Length::Fill)
            .height(Length::Fill)
            .style(theme::bg(theme::EDITOR_BG))
            .into()
    }

    /// Title and close button at the top of the left column.
    fn mode_switch(&self) -> Element<'static, Message> {
        row![text("History").size(13), space::horizontal()].align_y(Alignment::Center).into()
    }

    fn scm_left(&self) -> Element<'_, Message> {
        let s = &self.scm;
        let dim = theme::TEXT_DIM;
        let branch = self.git.as_ref().map(|g| g.branch.clone()).unwrap_or_else(|| "—".into());
        let busy = s.busy.is_some();
        let tool = |label: &'static str, msg: ScmMsg| {
            button(text(label).size(12))
                .padding([4, 8])
                .style(theme::flat(false))
                .on_press_maybe((!busy).then(|| scm(msg)))
        };
        let sync: Element<'_, Message> = match s.ahead_behind {
            Some((a, b)) => text(format!("↑{a} ↓{b}")).size(12).color(dim).into(),
            None => space::horizontal().width(Length::Shrink).into(),
        };

        let top = column![
            row![
                button(
                    row![icon(Icon::Branch, 14.0, dim), text(branch).size(13), icon(Icon::ChevronDown, 11.0, dim)]
                        .spacing(7)
                        .align_y(Alignment::Center),
                )
                .padding([5, 8])
                .style(theme::row(false))
                .on_press(scm(ScmMsg::BranchMenu(true))),
                space::horizontal(),
                sync,
            ]
            .align_y(Alignment::Center),
            row![tool("Fetch", ScmMsg::Fetch), tool("Pull", ScmMsg::Pull), tool("Push", ScmMsg::Push)].spacing(4),
        ]
        .spacing(4);

        let staged_count = s.staged().count();
        let changed = s.changes.len();
        let can_commit = !s.message.trim().is_empty();
        let files_word = |n: usize| format!("{n} file{}", if n == 1 { "" } else { "s" });
        let commit_box = column![
            text_input("Commit message", &s.message)
                .on_input(|t| scm(ScmMsg::Text(t)))
                .on_submit(scm(ScmMsg::Commit))
                .padding(8)
                .size(13)
                .style(theme::input),
            button(
                text(if staged_count > 0 {
                    format!("Commit {}", files_word(staged_count))
                } else if changed > 0 {
                    format!("Commit all {}", files_word(changed))
                } else {
                    "Commit message".into()
                })
                    .size(13)
                    .center()
                    .width(Length::Fill),
            )
            .width(Length::Fill)
            .padding([6, 8])
            .style(move |t: &Theme, st| {
                let mut style = theme::flat(can_commit)(t, st);
                if can_commit {
                    style.background = Some(Color { a: 0.28, ..theme::ACCENT }.into());
                    style.text_color = theme::TEXT;
                }
                style
            })
            .on_press_maybe(can_commit.then(|| scm(ScmMsg::Commit))),
            button(text("Commit & Push").size(13).center().width(Length::Fill))
                .width(Length::Fill)
                .padding([6, 8])
                .style(theme::flat(false))
                .on_press_maybe(can_commit.then(|| scm(ScmMsg::CommitPush))),
        ]
        .spacing(6);

        let file_row = |c: &Change, staged: bool| -> Element<'_, Message> {
            let status = if staged { c.staged } else { c.unstaged }.unwrap_or(Status::Modified);
            let (name, dir) = match c.path.rsplit_once('/') {
                Some((d, n)) => (n.to_string(), d.to_string()),
                None => (c.path.clone(), String::new()),
            };
            let selected = s.tab == ScmTab::Changes && s.selected.as_ref() == Some(&(c.path.clone(), staged));
            let toggle = if staged {
                icon_button(Icon::Minus, dim, false, scm(ScmMsg::Unstage(c.path.clone())))
            } else {
                icon_button(Icon::Plus, dim, false, scm(ScmMsg::Stage(c.path.clone())))
            };
            row![
                button(
                    row![
                        text(status.letter().to_string()).size(12).color(status_color(status)).width(Length::Fixed(12.0)),
                        text(name).size(13),
                        text(dir).size(11).color(theme::TEXT_FAINT),
                        space::horizontal(),
                        text(format!("+{}", c.added)).size(11).color(theme::ADDED),
                        text(format!("−{}", c.removed)).size(11).color(theme::REMOVED),
                    ]
                    .spacing(6)
                    .align_y(Alignment::Center),
                )
                .width(Length::Fill)
                .padding([4, 8])
                .style(theme::row(selected))
                .on_press(scm(ScmMsg::Select { path: c.path.clone(), staged })),
                toggle,
            ]
            .align_y(Alignment::Center)
            .into()
        };
        let section = |title: String, bulk: Option<(Icon, ScmMsg)>| -> Element<'_, Message> {
            let mut r = row![text(title).size(12).color(dim), space::horizontal()].align_y(Alignment::Center);
            if let Some((glyph, msg)) = bulk {
                r = r.push(icon_button(glyph, dim, false, scm(msg)));
            }
            container(r).padding(Padding { top: 8.0, right: 0.0, bottom: 2.0, left: 8.0 }).into()
        };

        let mut files = column![].spacing(1);
        if staged_count > 0 {
            files = files.push(section(format!("Staged Changes · {staged_count}"), Some((Icon::Minus, ScmMsg::UnstageAll))));
            for c in s.staged() {
                files = files.push(file_row(c, true));
            }
        }
        let unstaged_count = s.unstaged().count();
        if unstaged_count > 0 {
            files = files.push(section(format!("Changes · {unstaged_count}"), Some((Icon::Plus, ScmMsg::StageAll))));
            for c in s.unstaged() {
                files = files.push(file_row(c, false));
            }
        }
        if s.changes.is_empty() {
            files = files.push(container(text("No changes — the working tree is clean.").size(12).color(theme::TEXT_FAINT)).padding(10));
        }

        container(
            column![top, commit_box, crate::view::vscroll(column![files, self.commit_list()].spacing(4)).height(Length::Fill)]
                .spacing(10)
                .padding(10),
        )
        .width(Length::Fixed(self.scm_w))
        .height(Length::Fill)
        .style(theme::bg(theme::PANEL_BG))
        .into()
    }

    pub(crate) fn scm_diff(&self) -> Element<'_, Message> {
        if self.scm.tab == ScmTab::History {
            return self.history_detail();
        }
        let s = &self.scm;
        let Some((path, staged)) = &s.selected else {
            return container(text("Select a file to see its changes").size(13).color(theme::TEXT_FAINT))
                .center(Length::Fill)
                .into();
        };

        let header = container(
            row![
                text(if *staged { "staged" } else { "working tree" }).size(11).color(theme::TEXT_FAINT),
                space::horizontal(),
                button(text("Open file").size(12))
                    .padding([4, 8])
                    .style(theme::flat(false))
                    .on_press(scm(ScmMsg::OpenFile(path.clone()))),
            ]
            .spacing(10)
            .align_y(Alignment::Center)
            .padding([0, 12]),
        )
        .height(Length::Fixed(34.0))
        .align_y(Alignment::Center)
        .width(Length::Fill);

        container(column![header, divider_h(), diff_body(&s.diff, self.diff_pane_w())])
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }

    /// The commits, newest first, under the working changes.
    fn commit_list(&self) -> Element<'_, Message> {
        let s = &self.scm;
        let dim = theme::TEXT_DIM;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);

        let mut list = column![
            container(text(format!("History · {}", s.commits.len())).size(12).color(dim))
                .padding(Padding { top: 8.0, right: 0.0, bottom: 2.0, left: 8.0 })
        ]
        .spacing(1);
        if s.commits.is_empty() {
            list = list.push(container(text("No commits yet.").size(12).color(theme::TEXT_FAINT)).padding(10));
        }
        for c in &s.commits {
            list = list.push(self.chat_commit(c, now));
        }
        list.into()
    }

    /// Details of the selected commit: message, files, and the diff of one file.
    fn history_detail(&self) -> Element<'_, Message> {
        let s = &self.scm;
        let dim = theme::TEXT_DIM;
        let Some(commit) = s.selected_commit.as_ref().and_then(|sha| s.commits.iter().find(|c| &c.sha == sha)) else {
            return container(text("Select a commit to see what it changed").size(13).color(theme::TEXT_FAINT))
                .center(Length::Fill)
                .into();
        };
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);

        let head = column![
            text(commit.subject.clone()).size(15),
            row![
                text(commit.author.clone()).size(12).color(dim),
                text(time_ago(commit.time, now)).size(12).color(theme::TEXT_FAINT),
                text(commit.short.clone()).size(12).font(Font::MONOSPACE).color(theme::ACCENT),
            ]
            .spacing(10),
        ]
        .spacing(4);
        // The full message sits under the file list, clipped, so the files stay in view.
        let body: Element<'_, Message> = if commit.body.is_empty() {
            space::vertical().height(Length::Fixed(0.0)).into()
        } else {
            container(text(commit.body.clone()).size(12).color(dim)).max_height(84.0).clip(true).into()
        };

        let mut files = column![].spacing(1);
        for f in &s.commit_files {
            let (name, dir) = match f.path.rsplit_once('/') {
                Some((d, n)) => (n.to_string(), d.to_string()),
                None => (f.path.clone(), String::new()),
            };
            let selected = s.commit_file.as_deref() == Some(f.path.as_str());
            files = files.push(
                button(
                    row![
                        text(f.status.letter().to_string()).size(12).color(status_color(f.status)).width(Length::Fixed(12.0)),
                        text(name).size(13),
                        text(dir).size(11).color(theme::TEXT_FAINT),
                        space::horizontal(),
                        text(format!("+{}", f.added)).size(11).color(theme::ADDED),
                        text(format!("−{}", f.removed)).size(11).color(theme::REMOVED),
                    ]
                    .spacing(6)
                    .align_y(Alignment::Center),
                )
                .width(Length::Fill)
                .padding([4, 8])
                .style(theme::row(selected))
                .on_press(scm(ScmMsg::SelectCommitFile(f.path.clone()))),
            );
        }
        let files_label = text(format!(
            "{} file{} changed",
            s.commit_files.len(),
            if s.commit_files.len() == 1 { "" } else { "s" }
        ))
        .size(11)
        .color(theme::TEXT_FAINT);

        let top = container(
            column![
                head,
                files_label,
                container(crate::view::vscroll(files).height(Length::Shrink)).max_height(170.0),
                body,
            ]
            .spacing(8)
            .padding(12),
        )
        .width(Length::Fill);
        container(column![top, divider_h(), diff_body(&s.commit_diff, self.diff_pane_w())])
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }

    /// Overlay listing branches, anchored above the bottom bar.
    pub(crate) fn branch_menu(&self) -> Element<'_, Message> {
        let s = &self.scm;
        let mut list = column![].spacing(1);
        for b in &s.branches {
            list = list.push(list_row(
                row![
                    text(if b.current { "●" } else { " " }).size(10).color(theme::ACCENT).width(Length::Fixed(12.0)),
                    icon(Icon::Branch, 13.0, theme::TEXT_DIM),
                    text(b.name.clone()).size(13),
                ]
                .spacing(8)
                .align_y(Alignment::Center)
                .into(),
                b.current,
                scm(ScmMsg::Checkout(b.name.clone())),
            ));
        }
        let can_create = !s.new_branch.trim().is_empty();
        let create = row![
            text_input("New branch name", &s.new_branch)
                .on_input(|t| scm(ScmMsg::NewBranchText(t)))
                .on_submit(scm(ScmMsg::CreateBranch))
                .padding(7)
                .size(13)
                .style(theme::input),
            button(text("Create").size(12))
                .padding([6, 10])
                .style(theme::flat(can_create))
                .on_press_maybe(can_create.then(|| scm(ScmMsg::CreateBranch))),
        ]
        .spacing(6)
        .align_y(Alignment::Center);

        let panel = container(column![crate::view::vscroll(list).height(Length::Shrink), divider_h(), create].spacing(8).padding(8))
            .width(Length::Fixed(300.0))
            .max_height(420.0)
            .style(|_: &Theme| container::Style {
                background: Some(theme::PANEL_BG.into()),
                border: iced::Border { color: theme::SELECTED, width: 1.0, radius: 4.0.into() },
                ..Default::default()
            });
        stack![
            mouse_area(space::horizontal().width(Length::Fill).height(Length::Fill)).on_press(scm(ScmMsg::BranchMenu(false))),
            container(opaque(panel))
                .width(Length::Fill)
                .height(Length::Fill)
                .align_y(Alignment::End)
                .padding(Padding { bottom: BAR_H + 6.0, left: crate::view::RAIL_W + self.list_w + crate::view::HANDLE_W + 8.0, ..Padding::ZERO }),
        ]
        .into()
    }
}

/// A diff as coloured, numbered lines in a scrollable area.
/// Width of a monospace character at the diff's 12.5px size.
const DIFF_CHAR_W: f32 = 7.6;

/// Colour-coded diff lines that scroll sideways instead of wrapping. Every row is
/// as wide as the longest line (or `pane_w`, if that is wider) so tints line up.
fn diff_body(diff: &[DiffLine], pane_w: f32) -> Element<'static, Message> {
        let num = |n: Option<u32>| -> Element<'static, Message> {
            text(n.map(|n| n.to_string()).unwrap_or_default())
                .size(12)
                .font(Font::MONOSPACE)
                .color(theme::GUTTER)
                .align_x(Alignment::End)
                .width(Length::Fixed(44.0))
                .into()
        };
        let longest = diff.iter().take(MAX_DIFF_LINES).map(|l| l.text.chars().count()).max().unwrap_or(0);
        let row_w = (44.0 * 2.0 + 18.0 + 12.0 + longest as f32 * DIFF_CHAR_W + 24.0).max(pane_w - 20.0);
        let mut lines = column![];
        for l in diff.iter().take(MAX_DIFF_LINES) {
            let (tint, color, marker) = match l.kind {
                LineKind::Added => (Some(Color { a: 0.14, ..theme::ADDED }), theme::TEXT, "+"),
                LineKind::Removed => (Some(Color { a: 0.16, ..theme::REMOVED }), theme::TEXT, "−"),
                LineKind::Hunk => (Some(Color { a: 0.10, ..theme::ACCENT }), theme::ACCENT, " "),
                LineKind::Header => (None, theme::TEXT_DIM, " "),
                LineKind::Context => (None, theme::TEXT_DIM, " "),
            };
            let body = if l.kind == LineKind::Hunk {
                row![text(l.text.clone()).size(12.5).font(Font::MONOSPACE).color(color).wrapping(text::Wrapping::None)].padding([0, 12])
            } else {
                row![
                    num(l.old),
                    num(l.new),
                    text(marker).size(12.5).font(Font::MONOSPACE).color(color).width(Length::Fixed(18.0)),
                    text(l.text.clone()).size(12.5).font(Font::MONOSPACE).color(color).wrapping(text::Wrapping::None),
                ]
                .spacing(4)
            };
            let row_el = container(body).width(Length::Fixed(row_w)).padding([1, 0]);
            lines = lines.push(match tint {
                Some(c) => row_el.style(theme::bg(c)),
                None => row_el,
            });
        }
        if diff.len() > MAX_DIFF_LINES {
            lines = lines.push(container(text(format!("… {} more lines not shown", diff.len() - MAX_DIFF_LINES)).size(12).color(theme::TEXT_FAINT)).padding(10));
        }
        if diff.is_empty() {
            lines = lines.push(container(text("No textual changes to show.").size(12).color(theme::TEXT_FAINT)).padding(12));
        }

    scrollable(lines)
        .direction(scrollable::Direction::Both {
            vertical: crate::view::thin(),
            horizontal: crate::view::thin(),
        })
        .height(Length::Fill)
        .into()
}
