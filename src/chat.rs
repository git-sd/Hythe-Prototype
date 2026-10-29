//! Repository chat: a Discord-style channel whose messages are stored in git
//! (see `hythe_git::chat`) and shared with push and pull.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use hythe_git::ChatMessage;
use iced::widget::{button, column, container, row, scrollable, space, text, text_input};
use iced::{Alignment, Color, Element, Length, Padding, Task, Theme};

use crate::icons::{icon, Icon};
use crate::view::{divider_h, icon_button, TAB_H};
use crate::{theme, App, Message};

const HISTORY: usize = 300;

#[derive(Default)]
pub struct Chat {
    pub open: bool,
    pub messages: Vec<ChatMessage>,
    pub input: String,
    /// Files chosen to go with the next message.
    pub files: Vec<PathBuf>,
    /// What is running in the background, if anything.
    pub busy: Option<&'static str>,
    pub note: Option<String>,
    pub images: HashMap<String, iced::widget::image::Handle>,
    pub images_tried: HashSet<String>,
    /// Repository whose messages were last loaded.
    pub loaded_for: Option<PathBuf>,
    /// Closed from the welcome area, which shows the welcome menu again.
    pub dismissed: bool,
}

#[derive(Debug, Clone)]
pub enum ChatMsg {
    Show,
    Hide,
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
    Loaded(Result<Vec<ChatMessage>, String>),
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
            async move { tokio::task::spawn_blocking(move || repo.chat_messages(HISTORY)).await.unwrap_or_else(|e| Err(e.to_string())) },
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
                self.chat.open = true;
                self.chat.dismissed = false;
                self.scm.open = false;
                self.focus_term = false;
                return Task::batch([self.chat_load(), self.chat_sync()]);
            }
            ChatMsg::Hide => self.chat.open = false,
            ChatMsg::Dismiss => self.chat.dismissed = true,
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
                Ok(messages) => {
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

    /// The chat fills the editor area when asked for, and also stands in for the
    /// welcome screen while no file is open.
    pub(crate) fn chat_visible(&self) -> bool {
        self.chat.open || self.chat_in_welcome()
    }

    pub(crate) fn chat_in_welcome(&self) -> bool {
        self.repo.is_some() && !self.chat.dismissed && !self.scm.open && self.panes.list.len() == 1 && self.panes.list[0].tabs.is_empty()
    }

    /// `embedded`: shown in place of the welcome screen; its close button brings the welcome menu back.
    pub(crate) fn chat_view(&self, embedded: bool) -> Element<'_, Message> {
        let c = &self.chat;
        let dim = theme::TEXT_DIM;
        let msg = |m: ChatMsg| Message::Chat(m);
        let repo_name = self.git.as_ref().map(|g| g.repo.clone()).unwrap_or_else(|| "chat".into());

        let status: Element<'_, Message> = match (c.busy, &c.note) {
            (Some(b), _) => text(b).size(12).color(dim).wrapping(text::Wrapping::None).into(),
            (None, Some(n)) => text(n.clone()).size(12).color(theme::REMOVED).wrapping(text::Wrapping::None).into(),
            _ => text("Shared through git").size(12).color(theme::TEXT_FAINT).wrapping(text::Wrapping::None).into(),
        };
        let header = container(
            row![
                icon(Icon::Chat, 16.0, dim),
                text(repo_name.clone()).size(14).wrapping(text::Wrapping::None),
                // Long text is cut off rather than pushing Sync and the X out of view.
                container(status).width(Length::Fill).clip(true),
                button(text("Sync").size(12))
                    .padding([4, 10])
                    .style(theme::flat(false))
                    .on_press_maybe(c.busy.is_none().then(|| msg(ChatMsg::Sync))),
                icon_button(Icon::Close, theme::TEXT_FAINT, false, msg(if embedded { ChatMsg::Dismiss } else { ChatMsg::Hide })),
            ]
            .spacing(10)
            .align_y(Alignment::Center)
            .padding([0, 12]),
        )
        .height(Length::Fixed(TAB_H))
        .width(Length::Fill)
        .align_y(Alignment::Center);

        let now = now_secs();
        let mut list = column![].spacing(14).padding(Padding { top: 12.0, right: 16.0, bottom: 12.0, left: 16.0 });
        if c.messages.is_empty() {
            list = list.push(
                container(
                    column![
                        text(format!("Welcome to #{repo_name}")).size(18),
                        text("Messages and files you send here are saved in this repository and shared when you sync.")
                            .size(13)
                            .color(dim),
                    ]
                    .spacing(4),
                )
                .padding([40, 0]),
            );
        }
        for m in &c.messages {
            list = list.push(self.chat_message(m, now));
        }

        let mut attached = row![].spacing(6);
        for (i, f) in c.files.iter().enumerate() {
            let name = f.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            attached = attached.push(
                button(row![icon(Icon::File, 12.0, dim), text(name).size(12), icon(Icon::Close, 11.0, theme::TEXT_FAINT)].spacing(6).align_y(Alignment::Center))
                    .padding([3, 8])
                    .style(theme::flat(false))
                    .on_press(msg(ChatMsg::Unattach(i))),
            );
        }
        let can_send = !(c.input.trim().is_empty() && c.files.is_empty()) && c.busy != Some("Sending…");
        let composer = column![
            attached,
            row![
                icon_button(Icon::Paperclip, dim, false, msg(ChatMsg::Attach)),
                text_input(&format!("Message #{repo_name}"), &c.input)
                    .on_input(|s| Message::Chat(ChatMsg::Input(s)))
                    .on_submit(msg(ChatMsg::Send))
                    .padding(9)
                    .size(13)
                    .style(theme::input),
                button(icon(Icon::Send, 16.0, if can_send { theme::ACCENT } else { theme::TEXT_FAINT }))
                    .padding([6, 9])
                    .style(theme::flat(false))
                    .on_press_maybe(can_send.then(|| msg(ChatMsg::Send))),
            ]
            .spacing(6)
            .align_y(Alignment::Center),
        ]
        .spacing(6)
        .padding(12);

        // The 2px matches the focus line above the thread column, so the rules line up.
        container(column![space::vertical().height(Length::Fixed(2.0)), header, divider_h(), scrollable(list).anchor_bottom().width(Length::Fill).height(Length::Fill), divider_h(), composer])
            .width(Length::Fill)
            .height(Length::Fill)
            .style(theme::bg(theme::EDITOR_BG))
            .into()
    }

    fn chat_message<'a>(&'a self, m: &'a ChatMessage, now: i64) -> Element<'a, Message> {
        let c = &self.chat;
        let hue = m.author.bytes().fold(0u32, |a, b| a.wrapping_mul(31).wrapping_add(b as u32)) % 360;
        let avatar: Element<'a, Message> = match m.login.as_ref().and_then(|l| self.member_avatars.get(l)) {
            Some(h) => iced::widget::image(h.clone()).width(Length::Fixed(38.0)).height(Length::Fixed(38.0)).border_radius(19.0).into(),
            None => {
                let color = crate::view::hsv(hue as f32);
                container(text(m.author.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default()).size(15).color(Color::WHITE))
                    .center(Length::Fixed(38.0))
                    .style(move |_: &Theme| container::Style {
                        background: Some(color.into()),
                        border: iced::Border { radius: 19.0.into(), ..Default::default() },
                        ..Default::default()
                    })
                    .into()
            }
        };

        let mut who = row![text(m.author.clone()).size(14)].spacing(8).align_y(Alignment::Center);
        if let Some(l) = &m.login {
            who = who.push(text(format!("@{l}")).size(12).color(theme::TEXT_FAINT));
        }
        who = who.push(text(hythe_git::time_ago(m.time_ms / 1000, now)).size(11).color(theme::TEXT_FAINT));

        let mut body = column![who].spacing(3);
        if !m.body.trim().is_empty() {
            body = body.push(text(m.body.clone()).size(14));
        }
        for att in &m.attachments {
            let save = Message::Chat(ChatMsg::Save(att.entry.clone(), att.name.clone()));
            match c.images.get(&att.entry).filter(|_| att.is_image()) {
                Some(h) => {
                    body = body.push(
                        button(
                            container(iced::widget::image(h.clone()).content_fit(iced::ContentFit::Contain))
                                .width(Length::Fixed(320.0))
                                .height(Length::Fixed(220.0))
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
                            row![icon(Icon::File, 15.0, theme::TEXT_DIM), text(att.name.clone()).size(13), text("Save…").size(11).color(theme::TEXT_FAINT)]
                                .spacing(8)
                                .align_y(Alignment::Center),
                        )
                        .padding([6, 10])
                        .style(theme::flat(false))
                        .on_press(save),
                    );
                }
            }
        }
        row![avatar, body.width(Length::Fill)].spacing(12).into()
    }
}
