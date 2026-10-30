//! Repository chat: a Discord-style channel whose messages are stored in git
//! (see `hythe_git::chat`) and shared with push and pull.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use hythe_git::ChatMessage;
use iced::widget::{button, column, container, mouse_area, row, space, text, text_input};
use iced::{window, Alignment, Color, Element, Length, Padding, Task, Theme};

use crate::icons::{icon, Icon};
use crate::scm::ScmMsg;
use crate::view::{divider_h, icon_button, vscroll, Edge, TAB_H};
use crate::{theme, App, Handle, Message};

const HISTORY: usize = 300;

/// Where the chat sits when it is not in its own window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ChatMode {
    /// Floating over the editor, movable.
    #[default]
    Overlay,
    /// A column beside the editor.
    Docked,
}

/// What the chat panel is being drawn into.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Place {
    Docked,
    Overlay,
    Window,
}

#[derive(Default)]
pub struct Chat {
    pub messages: Vec<ChatMessage>,
    pub input: String,
    /// Files chosen to go with the next message.
    pub files: Vec<PathBuf>,
    /// What is running in the background, if anything.
    pub busy: Option<&'static str>,
    pub note: Option<String>,
    pub images: HashMap<String, iced::widget::image::Handle>,
    pub images_tried: HashSet<String>,
    /// Recent commits, shown in the feed between the messages.
    pub commits: Vec<hythe_git::Commit>,
    /// The commit card that is open, and the files it changed.
    pub expanded: Option<String>,
    pub expanded_files: Vec<hythe_git::CommitFile>,
    /// Repository whose messages were last loaded.
    pub loaded_for: Option<PathBuf>,
    /// Closed from the welcome area, which shows the welcome menu again.
    pub dismissed: bool,
}

#[derive(Debug, Clone)]
pub enum ChatMsg {
    Show,
    Hide,
    /// Switch between floating over the editor and docked beside it.
    ToggleDock,
    /// Open the chat in its own window.
    PopOut,
    /// Bring the chat back from its own window.
    PopIn,
    /// Close the chat shown in place of the welcome screen.
    Dismiss,
    Input(String),
    Attach,
    Unattach(usize),
    Send,
    /// Fetch, merge and push the chat branch.
    Sync,
    Posted(Result<(), String>),
    Synced(Result<String, String>),
    Loaded(Result<(Vec<ChatMessage>, Vec<hythe_git::Commit>), String>),
    /// Commit the text in the box (everything changed, if nothing is staged).
    Commit,
    /// Open a commit's changes in the full diff view.
    OpenCommit(String),
    /// Open or close a commit card in place.
    ToggleCommit(String),
    CommitFiles(String, Vec<hythe_git::CommitFile>),
    /// Open one file of a commit in the diff view.
    OpenCommitFile(String, String),
    /// Save an uploaded file somewhere (entry name, file name).
    Save(String, String),
    Saved(Result<String, String>),
    Image(String, Option<PathBuf>),
}

fn image_dir() -> Option<PathBuf> {
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))?;
    Some(base.join("hythe").join("chat"))
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

impl App {
    pub(crate) fn chat_load(&self) -> Task<Message> {
        let Some(repo) = self.repo.clone() else { return Task::none() };
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || {
                    let messages = repo.chat_messages(HISTORY)?;
                    Ok((messages, repo.log(60).unwrap_or_default()))
                })
                .await
                .unwrap_or_else(|e| Err(e.to_string()))
            },
            |r| Message::Chat(ChatMsg::Loaded(r)),
        )
    }

    pub(crate) fn chat_sync(&mut self) -> Task<Message> {
        let Some(repo) = self.repo.clone() else { return Task::none() };
        if self.chat.busy.is_some() {
            return Task::none();
        }
        self.chat.busy = Some("Syncing…");
        Task::perform(
            async move { tokio::task::spawn_blocking(move || repo.chat_sync()).await.unwrap_or_else(|e| Err(e.to_string())) },
            |r| Message::Chat(ChatMsg::Synced(r)),
        )
    }

    /// Logins of everyone who has posted, for fetching their pictures.
    pub(crate) fn chat_logins(&self) -> impl Iterator<Item = &String> {
        self.chat.messages.iter().filter_map(|m| m.login.as_ref())
    }

    pub(crate) fn chat_update(&mut self, msg: ChatMsg) -> Task<Message> {
        match msg {
            ChatMsg::Show => {
                if self.repo.is_none() {
                    self.status = "This folder isn't a git repository".into();
                    return Task::none();
                }
                self.chat.dismissed = false;
                self.focus_term = false;
                return Task::batch([self.chat_load(), self.chat_sync()]);
            }
            ChatMsg::Hide | ChatMsg::Dismiss => {
                self.chat.dismissed = true;
                if let Some(id) = self.chat_window.take() {
                    return window::close(id);
                }
            }
            ChatMsg::ToggleDock => {
                self.chat_mode = if self.chat_mode == ChatMode::Overlay { ChatMode::Docked } else { ChatMode::Overlay };
            }
            ChatMsg::PopOut => {
                if self.chat_window.is_some() {
                    return Task::none();
                }
                self.chat.dismissed = false;
                let (id, open) = window::open(window::Settings {
                    size: iced::Size::new(380.0, 640.0),
                    min_size: Some(iced::Size::new(260.0, 320.0)),
                    position: window::Position::Centered,
                    exit_on_close_request: false,
                    ..window::Settings::default()
                });
                self.chat_window = Some(id);
                return open.map(|_| Message::Noop);
            }
            ChatMsg::PopIn => {
                if let Some(id) = self.chat_window.take() {
                    return window::close(id);
                }
            }
            ChatMsg::Input(s) => self.chat.input = s,
            ChatMsg::Attach => {
                if let Some(files) = rfd::FileDialog::new().set_title("Attach files").pick_files() {
                    self.chat.files.extend(files);
                }
            }
            ChatMsg::Unattach(i) => {
                if i < self.chat.files.len() {
                    self.chat.files.remove(i);
                }
            }
            ChatMsg::Send => {
                let body = self.chat.input.trim().to_string();
                let Some(repo) = self.repo.clone() else { return Task::none() };
                if (body.is_empty() && self.chat.files.is_empty()) || self.chat.busy == Some("Sending…") {
                    return Task::none();
                }
                let files = std::mem::take(&mut self.chat.files);
                self.chat.input.clear();
                self.chat.note = None;
                self.chat.busy = Some("Sending…");
                return Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || repo.chat_post(&body, &files))
                            .await
                            .unwrap_or_else(|e| Err(e.to_string()))
                    },
                    |r| Message::Chat(ChatMsg::Posted(r)),
                );
            }
            ChatMsg::Posted(result) => {
                self.chat.busy = None;
                match result {
                    // Show it at once, then share it.
                    Ok(()) => {
                        if !self.chat_visible() {
                            self.status = "Message sent to chat".into();
                        }
                        return Task::batch([self.chat_load(), self.chat_sync()]);
                    }
                    Err(e) => {
                        self.status = e.lines().next().unwrap_or("send failed").to_string();
                        self.chat.note = Some(e);
                    }
                }
            }
            ChatMsg::Sync => return self.chat_sync(),
            ChatMsg::Synced(result) => {
                self.chat.busy = None;
                match result {
                    Ok(_) => self.chat.note = None,
                    Err(e) => self.chat.note = Some(e.lines().next().unwrap_or("sync failed").to_string()),
                }
                return self.chat_load();
            }
            ChatMsg::Loaded(result) => match result {
                Ok((messages, commits)) => {
                    self.chat.commits = commits;
                    let mut tasks = Vec::new();
                    if let Some(repo) = self.repo.clone() {
                        for att in messages.iter().flat_map(|m| &m.attachments).filter(|a| a.is_image()) {
                            if !self.chat.images_tried.insert(att.entry.clone()) {
                                continue;
                            }
                            let (repo, entry) = (repo.clone(), att.entry.clone());
                            let key = entry.clone();
                            tasks.push(Task::perform(
                                async move {
                                    tokio::task::spawn_blocking(move || {
                                        let dest = image_dir()?.join(&entry);
                                        std::fs::create_dir_all(dest.parent()?).ok()?;
                                        repo.chat_export(&entry, &dest).ok()?;
                                        Some(dest)
                                    })
                                    .await
                                    .ok()
                                    .flatten()
                                },
                                move |p| Message::Chat(ChatMsg::Image(key.clone(), p)),
                            ));
                        }
                    }
                    self.chat.messages = messages;
                    return Task::batch(tasks);
                }
                Err(e) => self.chat.note = Some(e),
            },
            ChatMsg::Commit => {
                let Some(repo) = self.repo.clone() else { return Task::none() };
                let text = self.chat.input.trim().to_string();
                if text.is_empty() {
                    self.status = "Write a commit message first".into();
                    return Task::none();
                }
                self.scm.reload(&repo);
                self.scm.message = text;
                if self.scm_commit(&repo) {
                    self.chat.input.clear();
                    return self.chat_load();
                }
            }
            ChatMsg::ToggleCommit(sha) => {
                if self.chat.expanded.as_ref() == Some(&sha) {
                    self.chat.expanded = None;
                    return Task::none();
                }
                self.chat.expanded = Some(sha.clone());
                self.chat.expanded_files.clear();
                let Some(repo) = self.repo.clone() else { return Task::none() };
                let key = sha.clone();
                return Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || repo.commit_files(&sha).unwrap_or_default())
                            .await
                            .unwrap_or_default()
                    },
                    move |files| Message::Chat(ChatMsg::CommitFiles(key.clone(), files)),
                );
            }
            ChatMsg::CommitFiles(sha, files) => {
                if self.chat.expanded.as_ref() == Some(&sha) {
                    self.chat.expanded_files = files;
                }
            }
            ChatMsg::OpenCommitFile(sha, path) => {
                let shown = self.scm_update(ScmMsg::Show);
                let selected = self.scm_update(ScmMsg::SelectCommit(sha));
                let file = self.scm_update(ScmMsg::SelectCommitFile(path));
                return Task::batch([shown, selected, file]);
            }
            ChatMsg::OpenCommit(sha) => {
                let shown = self.scm_update(ScmMsg::Show);
                let selected = self.scm_update(ScmMsg::SelectCommit(sha));
                return Task::batch([shown, selected]);
            }
            ChatMsg::Image(entry, path) => {
                if let Some(h) = path.and_then(|p| std::fs::read(p).ok()).map(iced::widget::image::Handle::from_bytes) {
                    self.chat.images.insert(entry, h);
                }
            }
            ChatMsg::Save(entry, name) => {
                let (Some(repo), Some(dest)) = (self.repo.clone(), rfd::FileDialog::new().set_file_name(&name).save_file()) else {
                    return Task::none();
                };
                return Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || {
                            repo.chat_export(&entry, &dest).map(|_| format!("Saved {}", dest.display()))
                        })
                        .await
                        .unwrap_or_else(|e| Err(e.to_string()))
                    },
                    |r| Message::Chat(ChatMsg::Saved(r)),
                );
            }
            ChatMsg::Saved(result) => self.status = result.unwrap_or_else(|e| e),
        }
        Task::none()
    }

    /// The chat is a side panel, on for every repository until closed.
    pub(crate) fn chat_visible(&self) -> bool {
        self.repo.is_some() && !self.chat.dismissed
    }

    /// The chat is a column beside the editor.
    pub(crate) fn chat_docked(&self) -> bool {
        self.chat_visible() && self.chat_window.is_none() && self.chat_mode == ChatMode::Docked
    }

    /// The chat floats over the window.
    pub(crate) fn chat_overlay(&self) -> bool {
        self.chat_visible() && self.chat_window.is_none() && self.chat_mode == ChatMode::Overlay
    }

    /// The floating chat, placed by its margins from the bottom-right corner.
    pub(crate) fn chat_overlay_layer(&self) -> Element<'_, Message> {
        container(iced::widget::opaque(self.chat_view_in(Place::Overlay)))
            .width(Length::Fill)
            .height(Length::Fill)
            .align_x(Alignment::End)
            .align_y(Alignment::End)
            .padding(Padding { right: self.chat_off.0, bottom: self.chat_off.1, ..Padding::ZERO })
            .into()
    }

    /// The chat panel. Drag its header to move it (overlay) or dock it on the other side.
    pub(crate) fn chat_view_in(&self, place: Place) -> Element<'_, Message> {
        let docked = place == Place::Docked;
        let c = &self.chat;
        let dim = theme::TEXT_DIM;
        let msg = |m: ChatMsg| Message::Chat(m);
        let repo_name = self.git.as_ref().map(|g| g.repo.clone()).unwrap_or_else(|| "chat".into());

        // Busy and error text replace the name for a moment; long text is cut off
        // rather than pushing Sync and the X out of view.
        let (title, title_color) = match (c.busy, &c.note) {
            (Some(b), _) => (b.to_string(), theme::TEXT_DIM),
            (None, Some(n)) => (n.clone(), theme::REMOVED),
            _ => (repo_name.clone(), theme::TEXT),
        };
        let header = container(
            row![
                if docked { self.header_left(Edge::Chat) } else { iced::widget::space::horizontal().width(Length::Shrink).into() },
                icon(Icon::Chat, 13.0, dim),
                container(text(title).size(12).color(title_color).wrapping(text::Wrapping::None))
                    .width(Length::Fill)
                    .clip(true),
                button(text("Sync").size(10))
                    .padding([2, 6])
                    .style(theme::flat(false))
                    .on_press_maybe(c.busy.is_none().then(|| msg(ChatMsg::Sync))),
                if place == Place::Window {
                    icon_button(Icon::Panels, theme::TEXT_FAINT, false, msg(ChatMsg::PopIn))
                } else {
                    icon_button(Icon::Panels, theme::TEXT_FAINT, docked, msg(ChatMsg::ToggleDock))
                },
                if place == Place::Window {
                    iced::widget::space::horizontal().width(Length::Shrink).into()
                } else {
                    icon_button(Icon::Expand, theme::TEXT_FAINT, false, msg(ChatMsg::PopOut))
                },
                icon_button(Icon::Close, theme::TEXT_FAINT, false, msg(ChatMsg::Hide)),
            ]
            .push(if docked { self.header_extra(Edge::Chat) } else { iced::widget::space::horizontal().width(Length::Shrink).into() })
            .spacing(6)
            .align_y(Alignment::Center)
            .padding([0, 8]),
        )
        .height(Length::Fixed(TAB_H))
        .width(Length::Fill)
        .align_y(Alignment::Center);
        let header: Element<'_, Message> = match place {
            Place::Docked => mouse_area(header).on_press(Message::DragStart(Handle::ChatDock)).interaction(iced::mouse::Interaction::Grab).into(),
            Place::Overlay => mouse_area(header).on_press(Message::DragStart(Handle::ChatMove)).interaction(iced::mouse::Interaction::Grab).into(),
            Place::Window => header.into(),
        };

        let branch = self.git.as_ref().map(|g| g.branch.clone()).unwrap_or_else(|| "—".into());
        let small = |label: &'static str, m: ScmMsg| {
            button(text(label).size(10)).padding([2, 6]).style(theme::flat(false)).on_press(Message::Scm(m))
        };
        let strip = container(
            row![
                icon(Icon::Branch, 12.0, dim),
                text(branch).size(11).wrapping(text::Wrapping::None),
                space::horizontal(),
                small("Pull", ScmMsg::Pull),
                small("Push", ScmMsg::Push),
            ]
            .spacing(6)
            .align_y(Alignment::Center)
            .padding([0, 8]),
        )
        .height(Length::Fixed(26.0))
        .align_y(Alignment::Center);

        let now = now_secs();
        let mut list = column![].spacing(20).padding(Padding { top: 14.0, right: 14.0, bottom: 14.0, left: 14.0 });
        if c.messages.is_empty() {
            list = list.push(
                container(
                    column![
                        text(format!("#{repo_name}")).size(13),
                        text("Messages and files are saved in this repo and shared when you sync.")
                            .size(11)
                            .color(dim),
                    ]
                    .spacing(3),
                )
                .padding([16, 0]),
            );
        }
        // Messages and commits in one feed, oldest first.
        enum Entry<'a> {
            Msg(&'a ChatMessage),
            Commit(&'a hythe_git::Commit),
        }
        let mut feed: Vec<(i64, Entry<'_>)> = c
            .messages
            .iter()
            .map(|m| (m.time_ms / 1000, Entry::Msg(m)))
            .chain(c.commits.iter().map(|k| (k.time, Entry::Commit(k))))
            .collect();
        feed.sort_by_key(|(t, _)| *t);
        for (_, entry) in feed {
            list = list.push(match entry {
                Entry::Msg(m) => self.chat_message(m, now),
                Entry::Commit(k) => self.chat_commit(k, now),
            });
        }

        let mut attached = column![].spacing(3);
        for (i, f) in c.files.iter().enumerate() {
            let name = f.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            attached = attached.push(
                button(row![icon(Icon::File, 11.0, dim), text(name).size(11).wrapping(text::Wrapping::None), icon(Icon::Close, 10.0, theme::TEXT_FAINT)].spacing(4).align_y(Alignment::Center))
                    .padding([2, 6])
                    .style(theme::flat(false))
                    .on_press(msg(ChatMsg::Unattach(i))),
            );
        }
        let can_send = !(c.input.trim().is_empty() && c.files.is_empty()) && c.busy != Some("Sending…");
        let changed = self.git.as_ref().map(|g| g.added + g.removed).unwrap_or(0) > 0;
        let can_commit = !c.input.trim().is_empty();
        let composer = column![
            attached,
            row![
                self.my_face(),
                text_input("Message", &c.input)
                    .on_input(|s| Message::Chat(ChatMsg::Input(s)))
                    .on_submit(msg(ChatMsg::Send))
                    .padding([4, 4])
                    .size(14)
                    .style(theme::bare),
                button(icon(Icon::Paperclip, 13.0, theme::TEXT_FAINT)).padding(3).style(theme::flat(false)).on_press(msg(ChatMsg::Attach)),
                button(icon(Icon::Send, 13.0, if can_send { theme::ACCENT } else { theme::TEXT_FAINT }))
                    .padding(3)
                    .style(theme::flat(false))
                    .on_press_maybe(can_send.then(|| msg(ChatMsg::Send))),
            ]
            .spacing(3)
            .align_y(Alignment::Center),
            row![
                button(
                    text(if changed { "Uncommitted changes" } else { "Nothing to commit" })
                        .size(10)
                        .color(if changed { theme::ACCENT } else { theme::TEXT_FAINT })
                        .wrapping(text::Wrapping::None),
                )
                .padding([2, 4])
                .style(theme::flat(false))
                .on_press_maybe(changed.then(|| Message::Scm(ScmMsg::Show))),
                space::horizontal(),
                button(text("Commit").size(10))
                    .padding([2, 8])
                    .style(theme::flat(false))
                    .on_press_maybe(can_commit.then(|| msg(ChatMsg::Commit))),
            ]
            .align_y(Alignment::Center),
        ]
        .spacing(6)
        .padding([10, 14]);

        let body = column![header, divider_h(), strip, divider_h(), vscroll(list).anchor_bottom().width(Length::Fill).height(Length::Fill), divider_h(), composer];
        match place {
            Place::Docked => container(body).width(Length::Fixed(self.chat_w)).height(Length::Fill).style(theme::bg(theme::PANEL_BG)).into(),
            Place::Window => container(body).width(Length::Fill).height(Length::Fill).style(theme::bg(theme::PANEL_BG)).into(),
            Place::Overlay => {
                let height = (self.win.height - self.chat_off.1 - 60.0).clamp(240.0, 640.0);
                container(body)
                    .width(Length::Fixed(self.chat_w))
                    .height(Length::Fixed(height))
                    .clip(true)
                    .style(|_: &Theme| container::Style {
                        background: Some(theme::PANEL_BG.into()),
                        border: iced::Border { color: theme::SELECTED, width: 1.0, radius: 8.0.into() },
                        shadow: iced::Shadow {
                            color: Color { a: 0.45, ..Color::BLACK },
                            offset: iced::Vector::new(0.0, 6.0),
                            blur_radius: 18.0,
                        },
                        ..Default::default()
                    })
                    .into()
            }
        }
    }

    /// Your own picture, beside the message box.
    fn my_face(&self) -> Element<'_, Message> {
        match &self.avatar {
            Some(h) => iced::widget::image(h.clone()).width(Length::Fixed(22.0)).height(Length::Fixed(22.0)).border_radius(11.0).into(),
            None => self.chat_face(&self.user, None, None, 0),
        }
    }

    /// A round picture (or initial) for someone; hovering it names them.
    fn chat_face<'a>(&'a self, name: &str, login: Option<&String>, when: Option<i64>, now: i64) -> Element<'a, Message> {
        const SIZE: f32 = 22.0;
        let hue = name.bytes().fold(0u32, |a, b| a.wrapping_mul(31).wrapping_add(b as u32)) % 360;
        let face: Element<'a, Message> = match login.and_then(|l| self.member_avatars.get(l)) {
            Some(h) => iced::widget::image(h.clone()).width(Length::Fixed(SIZE)).height(Length::Fixed(SIZE)).border_radius(SIZE / 2.0).into(),
            None => {
                let color = crate::view::hsv(hue as f32);
                container(text(name.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default()).size(11).color(Color::WHITE))
                    .center(Length::Fixed(SIZE))
                    .style(move |_: &Theme| container::Style {
                        background: Some(color.into()),
                        border: iced::Border { radius: (SIZE / 2.0).into(), ..Default::default() },
                        ..Default::default()
                    })
                    .into()
            }
        };
        let label = match when {
            Some(t) => format!("{name} · {}", hythe_git::time_ago(t, now)),
            None => name.to_string(),
        };
        iced::widget::tooltip(
            face,
            container(text(label).size(11)).padding([3, 6]).style(|_: &Theme| container::Style {
                background: Some(theme::BAR_BG.into()),
                border: iced::Border { color: theme::SELECTED, width: 1.0, radius: 2.0.into() },
                ..Default::default()
            }),
            iced::widget::tooltip::Position::Right,
        )
        .gap(4)
        .into()
    }

    /// A commit in the feed: a heading, its description under it and the hash, in a
    /// card like a code block. Click for its changes.
    fn chat_commit<'a>(&'a self, k: &'a hythe_git::Commit, now: i64) -> Element<'a, Message> {
        let login = self.members.iter().find(|m| m.name == k.author).and_then(|m| m.login.as_ref());
        let avatar = self.chat_face(&k.author, login, Some(k.time), now);
        let open = self.chat.expanded.as_ref() == Some(&k.sha);
        let mut body = column![text(k.subject.clone()).size(14).color(theme::TEXT).line_height(1.4)].spacing(3);
        let detail = k.body.lines().filter(|l| !l.starts_with("Co-Authored-By:")).collect::<Vec<_>>().join("\n");
        if !detail.trim().is_empty() {
            let para = text(detail.trim().to_string()).size(13).color(theme::TEXT_DIM).line_height(1.4);
            // Closed: a short preview. Open: the whole description.
            body = body.push(if open { Element::from(para) } else { container(para).max_height(72.0).clip(true).into() });
        }
        body = body.push(
            row![
                text(k.short.clone()).size(11).font(iced::Font::MONOSPACE).color(theme::ACCENT),
                space::horizontal(),
                icon(if open { Icon::ChevronDown } else { Icon::ChevronRight }, 11.0, theme::TEXT_FAINT),
            ]
            .align_y(Alignment::Center),
        );
        if open {
            let mut files = column![].spacing(1);
            if self.chat.expanded_files.is_empty() {
                files = files.push(text("Loading files…").size(11).color(theme::TEXT_FAINT));
            }
            for f in &self.chat.expanded_files {
                let (name, dir) = match f.path.rsplit_once('/') {
                    Some((d, n)) => (n.to_string(), d.to_string()),
                    None => (f.path.clone(), String::new()),
                };
                let tint = match f.status {
                    hythe_git::Status::Added | hythe_git::Status::Untracked => theme::ADDED,
                    hythe_git::Status::Deleted | hythe_git::Status::Conflicted => theme::REMOVED,
                    _ => theme::ACCENT,
                };
                files = files.push(
                    button(
                        row![
                            text(f.status.letter().to_string()).size(11).color(tint).width(Length::Fixed(10.0)),
                            text(name).size(12).wrapping(text::Wrapping::None),
                            container(text(dir).size(10).color(theme::TEXT_FAINT).wrapping(text::Wrapping::None))
                                .width(Length::Fill)
                                .clip(true),
                            text(format!("+{}", f.added)).size(10).color(theme::ADDED),
                            text(format!("−{}", f.removed)).size(10).color(theme::REMOVED),
                        ]
                        .spacing(6)
                        .align_y(Alignment::Center),
                    )
                    .width(Length::Fill)
                    .padding([3, 4])
                    .style(theme::row(false))
                    .on_press(Message::Chat(ChatMsg::OpenCommitFile(k.sha.clone(), f.path.clone()))),
                );
            }
            body = body.push(container(crate::view::divider_h()).padding([4, 0]));
            body = body.push(files);
        }
        let card = button(body)
            .width(Length::Fill)
            .padding([8, 10])
            .style(|_: &Theme, status| button::Style {
                background: Some(if matches!(status, button::Status::Hovered) { theme::HOVER } else { theme::BAR_BG }.into()),
                text_color: theme::TEXT,
                border: iced::Border { color: theme::SELECTED, width: 1.0, radius: 4.0.into() },
                ..button::Style::default()
            })
            .on_press(Message::Chat(ChatMsg::ToggleCommit(k.sha.clone())));
        row![avatar, card].spacing(12).into()
    }

    /// A message: plain text beside a small picture, no name line.
    fn chat_message<'a>(&'a self, m: &'a ChatMessage, now: i64) -> Element<'a, Message> {
        let c = &self.chat;
        let avatar = self.chat_face(&m.author, m.login.as_ref(), Some(m.time_ms / 1000), now);
        let mut body = column![].spacing(6);
        if !m.body.trim().is_empty() {
            body = body.push(text(m.body.clone()).size(14).line_height(1.45));
        }
        for att in &m.attachments {
            let save = Message::Chat(ChatMsg::Save(att.entry.clone(), att.name.clone()));
            match c.images.get(&att.entry).filter(|_| att.is_image()) {
                Some(h) => {
                    body = body.push(
                        button(
                            container(iced::widget::image(h.clone()).content_fit(iced::ContentFit::Contain))
                                .width(Length::Fill)
                                .height(Length::Fixed(160.0))
                                .align_x(Alignment::Start),
                        )
                        .padding(0)
                        .style(|_: &Theme, _| button::Style::default())
                        .on_press(save),
                    );
                }
                None => {
                    body = body.push(
                        button(
                            row![icon(Icon::File, 13.0, theme::TEXT_DIM), text(att.name.clone()).size(12).wrapping(text::Wrapping::None)]
                                .spacing(6)
                                .align_y(Alignment::Center),
                        )
                        .padding([4, 8])
                        .style(theme::flat(false))
                        .on_press(save),
                    );
                }
            }
        }
        row![avatar, body.width(Length::Fill)].spacing(12).into()
    }
}
