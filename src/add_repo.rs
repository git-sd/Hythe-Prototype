//! "Add a repository": open a folder that's already on this computer, or
//! clone one from GitHub (or any git URL).

use std::path::PathBuf;

use iced::widget::{button, column, container, mouse_area, opaque, row, scrollable, space, stack, text, text_input};
use iced::{Alignment, Color, Element, Length, Task, Theme};

use crate::github::RepoEntry;
use crate::icons::{icon, Icon};
use crate::view::icon_button;
use crate::{theme, App, Message};

#[derive(Default)]
pub struct AddRepo {
    pub open: bool,
    /// The clone form is showing.
    pub clone: bool,
    pub url: String,
    pub dest: Option<PathBuf>,
    pub busy: bool,
    pub error: Option<String>,
    /// The account's most recently pushed repositories.
    pub repos: Vec<RepoEntry>,
    pub loading: bool,
    /// Why the list is empty, when it is.
    pub list_note: Option<String>,
}

#[derive(Debug, Clone)]
pub enum AddRepoMsg {
    Open,
    Close,
    /// Pick a folder already on this computer.
    Local,
    ShowClone,
    Url(String),
    ChooseDest,
    Clone,
    Done(Result<PathBuf, String>),
    Repos(Result<Vec<RepoEntry>, String>),
    /// Fill the form from the n-th listed repository.
    Pick(usize),
}

impl App {
    /// Where clones go unless told otherwise: next to the open repository.
    fn default_clone_dir(&self) -> Option<PathBuf> {
        let beside = self
            .root
            .as_ref()
            .and_then(|r| r.path.parent())
            .map(|p| p.to_path_buf());
        beside.or_else(|| std::env::var_os("USERPROFILE").map(|h| PathBuf::from(h).join("Documents")).filter(|p| p.is_dir()))
    }

    pub(crate) fn add_repo_update(&mut self, msg: AddRepoMsg) -> Task<Message> {
        match msg {
            AddRepoMsg::Open => {
                self.add_repo = AddRepo { open: true, dest: self.default_clone_dir(), ..AddRepo::default() };
            }
            AddRepoMsg::Close => {
                if !self.add_repo.busy {
                    self.add_repo.open = false;
                }
            }
            AddRepoMsg::Local => {
                self.add_repo.open = false;
                return self.update(Message::OpenFolderDialog);
            }
            AddRepoMsg::ShowClone => {
                self.add_repo.clone = true;
                if self.add_repo.repos.is_empty() && !self.add_repo.loading {
                    self.add_repo.loading = true;
                    self.add_repo.list_note = None;
                    let login = self.github_login();
                    return Task::perform(
                        async move {
                            tokio::task::spawn_blocking(move || crate::github::recent_repos(login.as_deref()))
                                .await
                                .unwrap_or_else(|e| Err(e.to_string()))
                        },
                        |r| Message::AddRepo(AddRepoMsg::Repos(r)),
                    );
                }
            }
            AddRepoMsg::Repos(result) => {
                self.add_repo.loading = false;
                match result {
                    Ok(list) => {
                        self.add_repo.list_note = list.is_empty().then(|| "No repositories yet".to_string());
                        self.add_repo.repos = list;
                    }
                    Err(e) => self.add_repo.list_note = Some(e),
                }
            }
            AddRepoMsg::Pick(i) => {
                if let Some(r) = self.add_repo.repos.get(i) {
                    self.add_repo.url = r.clone_url.clone();
                    self.add_repo.error = None;
                }
            }
            AddRepoMsg::Url(s) => {
                self.add_repo.url = s;
                self.add_repo.error = None;
            }
            AddRepoMsg::ChooseDest => {
                if let Some(p) = rfd::FileDialog::new().pick_folder() {
                    self.add_repo.dest = Some(p);
                    self.add_repo.error = None;
                }
            }
            AddRepoMsg::Clone => {
                let state = &mut self.add_repo;
                if state.busy {
                    return Task::none();
                }
                let url = hythe_git::clone_url(&state.url);
                let name = hythe_git::repo_dir_name(&url);
                let (Some(dest), false) = (state.dest.clone(), name.is_empty()) else {
                    state.error = Some("Enter a repository and choose where to put it".into());
                    return Task::none();
                };
                let target = dest.join(name);
                state.busy = true;
                state.error = None;
                self.status = format!("Cloning {url}…");
                return Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || hythe_git::clone_repo(&url, &target).map(|_| target))
                            .await
                            .unwrap_or_else(|e| Err(e.to_string()))
                    },
                    |r| Message::AddRepo(AddRepoMsg::Done(r)),
                );
            }
            AddRepoMsg::Done(Ok(path)) => {
                self.add_repo = AddRepo::default();
                self.status = format!("Cloned into {}", path.display());
                self.open_folder(path);
            }
            AddRepoMsg::Done(Err(e)) => {
                self.add_repo.busy = false;
                // Keep git's first useful line; the rest is progress noise.
                self.add_repo.error = Some(
                    e.lines()
                        .find(|l| l.contains("fatal") || l.contains("error") || l.contains("already exists"))
                        .or_else(|| e.lines().next())
                        .unwrap_or("clone failed")
                        .to_string(),
                );
            }
        }
        Task::none()
    }

    pub(crate) fn add_repo_dialog(&self) -> Element<'_, Message> {
        let s = &self.add_repo;
        let dim = theme::TEXT_DIM;
        let msg = |m: AddRepoMsg| Message::AddRepo(m);

        let choice = |glyph: Icon, title: &'static str, sub: &'static str, m: AddRepoMsg, selected: bool| {
            button(
                row![
                    icon(glyph, 18.0, dim),
                    column![text(title).size(14), text(sub).size(12).color(theme::TEXT_DIM)].spacing(2),
                ]
                .spacing(12)
                .align_y(Alignment::Center),
            )
            .width(Length::Fill)
            .padding([10, 12])
            .style(theme::row(selected))
            .on_press(msg(m))
        };

        let mut body = column![
            row![
                text("Add a repository").size(15),
                space::horizontal(),
                icon_button(Icon::Close, theme::TEXT_FAINT, false, msg(AddRepoMsg::Close)),
            ]
            .align_y(Alignment::Center),
            choice(Icon::Folder, "Open a folder", "A repository already on this computer", AddRepoMsg::Local, false),
            choice(Icon::Branch, "Clone from GitHub", "Download a repository by URL or owner/name", AddRepoMsg::ShowClone, s.clone),
        ]
        .spacing(6);

        if s.clone {
            let dest = s.dest.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| "No folder chosen".into());
            let can_clone = !s.busy && !s.url.trim().is_empty() && s.dest.is_some();
            body = body.push(container(crate::view::divider_h()).padding([6, 0]));
            body = body.push(text("Your recent repositories").size(11).color(dim));
            if s.loading {
                body = body.push(text("Loading from GitHub…").size(12).color(theme::TEXT_FAINT));
            } else if s.repos.is_empty() {
                let note = s.list_note.clone().unwrap_or_default();
                body = body.push(text(note).size(12).color(theme::TEXT_FAINT));
                body = body.push(
                    text("Run `gh auth login` to also list private repositories.").size(11).color(theme::TEXT_FAINT),
                );
            } else {
                let wanted = hythe_git::clone_url(&s.url);
                let mut list = column![].spacing(1);
                for (i, r) in s.repos.iter().enumerate() {
                    let mut name = row![text(r.full_name.clone()).size(13)].spacing(6).align_y(Alignment::Center);
                    if r.private {
                        name = name.push(text("private").size(10).color(theme::TEXT_FAINT));
                    }
                    let sub = if r.description.is_empty() { r.pushed.clone() } else { format!("{} · {}", r.pushed, r.description) };
                    list = list.push(
                        button(column![name, text(sub).size(11).color(theme::TEXT_DIM).wrapping(text::Wrapping::None)].spacing(1))
                            .width(Length::Fill)
                            .padding([5, 8])
                            .style(theme::row(wanted == r.clone_url))
                            .on_press(msg(AddRepoMsg::Pick(i))),
                    );
                }
                body = body.push(scrollable(list).height(Length::Fixed(190.0)));
            }
            body = body.push(text("Or clone with a link").size(11).color(dim));
            body = body.push(
                text_input("owner/name or https://github.com/owner/name", &s.url)
                    .on_input(|t| Message::AddRepo(AddRepoMsg::Url(t)))
                    .on_submit(msg(AddRepoMsg::Clone))
                    .padding(8)
                    .size(13)
                    .style(theme::input),
            );
            body = body.push(text("Clone into").size(11).color(dim));
            body = body.push(
                row![
                    container(text(dest).size(12).color(theme::TEXT).wrapping(text::Wrapping::None))
                        .width(Length::Fill)
                        .clip(true)
                        .padding([7, 8])
                        .style(|_: &Theme| container::Style {
                            background: Some(theme::EDITOR_BG.into()),
                            border: iced::Border { color: theme::SELECTED, width: 1.0, radius: 3.0.into() },
                            ..Default::default()
                        }),
                    button(text("Choose…").size(12))
                        .padding([7, 10])
                        .style(theme::flat(false))
                        .on_press(msg(AddRepoMsg::ChooseDest)),
                ]
                .spacing(6)
                .align_y(Alignment::Center),
            );
            if let Some(e) = &s.error {
                body = body.push(text(e.clone()).size(12).color(theme::REMOVED));
            }
            body = body.push(
                button(
                    text(if s.busy { "Cloning…" } else { "Clone" }).size(13).center().width(Length::Fill),
                )
                .width(Length::Fill)
                .padding([7, 10])
                .style(move |t: &Theme, st| {
                    let mut style = theme::flat(can_clone)(t, st);
                    if can_clone {
                        style.background = Some(Color { a: 0.28, ..theme::ACCENT }.into());
                        style.text_color = theme::TEXT;
                    }
                    style
                })
                .on_press_maybe(can_clone.then(|| msg(AddRepoMsg::Clone))),
            );
            body = body.push(
                text("Uses your git credentials, so private repositories work if git can already reach them.")
                    .size(11)
                    .color(theme::TEXT_FAINT),
            );
        }

        let panel = container(body.padding(14))
            .width(Length::Fixed(460.0))
            .style(|_: &Theme| container::Style {
                background: Some(theme::PANEL_BG.into()),
                border: iced::Border { color: theme::SELECTED, width: 1.0, radius: 4.0.into() },
                ..Default::default()
            });
        stack![
            mouse_area(space::horizontal().width(Length::Fill).height(Length::Fill)).on_press(msg(AddRepoMsg::Close)),
            container(opaque(panel)).center(Length::Fill),
        ]
        .into()
    }
}
