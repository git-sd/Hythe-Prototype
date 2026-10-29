//! Drawing for source control: the Changes view and the branch picker.

use hythe_git::{time_ago, Change, DiffLine, LineKind, Status};
use iced::widget::{button, column, container, mouse_area, opaque, row, scrollable, space, stack, text, text_input};
use iced::{Alignment, Color, Element, Font, Length, Padding, Theme};

use crate::icons::{icon, Icon};
use crate::scm::{ScmMsg, ScmTab};
use crate::view::{divider_h, divider_v, icon_button, list_row, BAR_H, SIDEBAR_W, TAB_H};
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
    pub(crate) fn scm_view(&self) -> Element<'_, Message> {
        let header = container(
            row![
                row![
                    tab_button("Changes", self.scm.tab == ScmTab::Changes, ScmTab::Changes),
                    tab_button("History", self.scm.tab == ScmTab::History, ScmTab::History),
                ]
                .spacing(2)
                .padding([0, 8]),
                space::horizontal(),
                icon_button(Icon::Close, theme::TEXT_FAINT, false, scm(ScmMsg::Hide)),
            ]
            .align_y(Alignment::Center)
            .padding([0, 6]),
        )
        .width(Length::Fill)
        .height(Length::Fixed(TAB_H))
        .align_y(Alignment::Center)
        .style(theme::bg(theme::TAB_BAR_BG));

        container(column![header, divider_h(), row![self.scm_left(), divider_v(), self.scm_diff()]])
            .width(Length::Fill)
            .height(Length::Fill)
            .style(theme::bg(theme::EDITOR_BG))
            .into()
    }

    fn scm_left(&self) -> Element<'_, Message> {
        if self.scm.tab == ScmTab::History {
            return self.history_left();
        }
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
        let can_commit = staged_count > 0 && !s.message.trim().is_empty();
        let commit_box = column![
            text_input("Commit message", &s.message)
                .on_input(|t| scm(ScmMsg::Text(t)))
                .on_submit(scm(ScmMsg::Commit))
                .padding(8)
                .size(13)
                .style(theme::input),
            button(
                text(if staged_count > 0 { format!("Commit {staged_count} file{}", if staged_count == 1 { "" } else { "s" }) } else { "Commit".into() })
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
        ]
        .spacing(6);

        let file_row = |c: &Change, staged: bool| -> Element<'_, Message> {
            let status = if staged { c.staged } else { c.unstaged }.unwrap_or(Status::Modified);
            let (name, dir) = match c.path.rsplit_once('/') {
                Some((d, n)) => (n.to_string(), d.to_string()),
                None => (c.path.clone(), String::new()),
            };
            let selected = s.selected.as_ref() == Some(&(c.path.clone(), staged));
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
            column![top, commit_box, scrollable(files).height(Length::Fill)]
                .spacing(10)
                .padding(10),
        )
        .width(Length::Fixed(340.0))
        .height(Length::Fill)
        .style(theme::bg(theme::PANEL_BG))
        .into()
    }

    fn scm_diff(&self) -> Element<'_, Message> {
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
                text(path.clone()).size(13),
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

        container(column![header, divider_h(), diff_body(&s.diff)])
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }

    /// The commit list.
    fn history_left(&self) -> Element<'_, Message> {
        let s = &self.scm;
        let dim = theme::TEXT_DIM;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let branch = self.git.as_ref().map(|g| g.branch.clone()).unwrap_or_else(|| "—".into());

        let mut list = column![].spacing(1);
        if s.commits.is_empty() {
            list = list.push(container(text("No commits yet.").size(12).color(theme::TEXT_FAINT)).padding(10));
        }
        for c in &s.commits {
            let selected = s.selected_commit.as_deref() == Some(c.sha.as_str());
            list = list.push(
                button(
                    column![
                        container(text(c.subject.clone()).size(13).wrapping(text::Wrapping::None)).width(Length::Fill).clip(true),
                        row![
                            text(c.short.clone()).size(11).font(Font::MONOSPACE).color(theme::ACCENT),
                            text(c.author.clone()).size(11).color(dim),
                            text(time_ago(c.time, now)).size(11).color(theme::TEXT_FAINT),
                        ]
                        .spacing(8),
                    ]
                    .spacing(3),
                )
                .width(Length::Fill)
                .padding([6, 8])
                .style(theme::row(selected))
                .on_press(scm(ScmMsg::SelectCommit(c.sha.clone()))),
            );
        }

        let top = row![
            icon(Icon::Branch, 14.0, dim),
            text(branch).size(13),
            space::horizontal(),
            text(format!("{} commits", s.commits.len())).size(11).color(theme::TEXT_FAINT),
        ]
        .spacing(8)
        .align_y(Alignment::Center);

        container(column![top, scrollable(list).height(Length::Fill)].spacing(8).padding(10))
            .width(Length::Fixed(340.0))
            .height(Length::Fill)
            .style(theme::bg(theme::PANEL_BG))
            .into()
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

        let mut head = column![
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
                container(scrollable(files).height(Length::Shrink)).max_height(170.0),
                body,
            ]
            .spacing(8)
            .padding(12),
        )
        .width(Length::Fill);
        container(column![top, divider_h(), diff_body(&s.commit_diff)])
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

        let panel = container(column![scrollable(list).height(Length::Shrink), divider_h(), create].spacing(8).padding(8))
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
                .padding(Padding { bottom: BAR_H + 6.0, left: SIDEBAR_W + 8.0, ..Padding::ZERO }),
        ]
        .into()
    }
}

/// A diff as coloured, numbered lines in a scrollable area.
fn diff_body(diff: &[DiffLine]) -> Element<'static, Message> {
        let num = |n: Option<u32>| -> Element<'static, Message> {
            text(n.map(|n| n.to_string()).unwrap_or_default())
                .size(12)
                .font(Font::MONOSPACE)
                .color(theme::GUTTER)
                .align_x(Alignment::End)
                .width(Length::Fixed(44.0))
                .into()
        };
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
                row![text(l.text.clone()).size(12.5).font(Font::MONOSPACE).color(color)].padding([0, 12])
            } else {
                row![
                    num(l.old),
                    num(l.new),
                    text(marker).size(12.5).font(Font::MONOSPACE).color(color).width(Length::Fixed(18.0)),
                    text(l.text.clone()).size(12.5).font(Font::MONOSPACE).color(color),
                ]
                .spacing(4)
            };
            let row_el = container(body).width(Length::Fill).padding([1, 0]);
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

    scrollable(lines).height(Length::Fill).into()
}

/// A tab in the Source Control header.
fn tab_button(label: &'static str, selected: bool, tab: ScmTab) -> Element<'static, Message> {
    button(text(label).size(13).color(if selected { theme::TEXT } else { theme::TEXT_DIM }))
        .padding([6, 12])
        .style(theme::flat(selected))
        .on_press(scm(ScmMsg::Tab(tab)))
        .into()
}