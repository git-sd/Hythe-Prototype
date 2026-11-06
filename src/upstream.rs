//! Watches the remote: every 15 seconds fetch quietly and, when new
//! commits have landed upstream, offer to pull them in.

use iced::widget::{button, container, row, space, text};
use iced::{Alignment, Element, Length, Padding, Task};

use crate::{theme, App, Message};

/// What is waiting on the remote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Upstream {
    pub behind: u32,
    /// Remote branch tip, so "Later" only hides this particular state.
    pub head: String,
    pub files: Vec<String>,
    pub last: String,
}

impl App {
    pub(crate) fn upstream_task(&mut self) -> Task<Message> {
        // Don't overlap checks, and don't fetch under a push/pull already running.
        if self.upstream_busy || self.scm.busy.is_some() {
            return Task::none();
        }
        let Some(repo) = self.repo.clone() else {
            return Task::none();
        };
        self.upstream_busy = true;
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || {
                    repo.fetch_quiet().ok()?;
                    let (_, behind) = repo.ahead_behind()?;
                    if behind == 0 {
                        return None;
                    }
                    Some(Upstream {
                        behind,
                        head: repo.upstream_head()?,
                        files: repo.incoming_files(),
                        last: repo.upstream_last_commit().unwrap_or_default(),
                    })
                })
                .await
                .ok()
                .flatten()
            },
            Message::UpstreamChecked,
        )
    }

    pub(crate) fn upstream_checked(&mut self, found: Option<Upstream>) {
        self.upstream_busy = false;
        self.upstream = found;
    }

    pub(crate) fn upstream_update(&mut self) -> Task<Message> {
        self.upstream = None;
        self.update_inner(Message::Scm(crate::scm::ScmMsg::Pull))
    }

    pub(crate) fn upstream_later(&mut self) {
        self.upstream_later = self.upstream.as_ref().map(|u| u.head.clone());
        self.upstream = None;
    }

    /// The "N new commits on the remote, update?" strip, when there is one to show.
    pub(crate) fn upstream_banner(&self) -> Option<Element<'_, Message>> {
        let u = self.upstream.as_ref()?;
        if self.upstream_later.as_deref() == Some(u.head.as_str()) {
            return None;
        }
        let what = if u.behind == 1 { "1 new commit".to_string() } else { format!("{} new commits", u.behind) };
        let mut detail = match u.files.len() {
            0 => String::new(),
            1 => format!(" · {}", u.files[0]),
            n => format!(" · {n} files"),
        };
        if !u.last.is_empty() {
            detail.push_str(&format!(" · {}", u.last));
        }
        let content = row![
            text(format!("{what} on the remote")).size(12).color(theme::TEXT),
            text(detail).size(12).color(theme::TEXT_FAINT).wrapping(text::Wrapping::None),
            space::horizontal(),
            button(text("Update").size(12)).padding(Padding::from([2, 10])).on_press(Message::UpstreamUpdate),
            button(text("Later").size(12)).padding(Padding::from([2, 10])).style(theme::flat(false)).on_press(Message::UpstreamLater),
        ]
        .spacing(10)
        .align_y(Alignment::Center);
        Some(
            container(content)
                .padding(Padding::from([5, 12]))
                .width(Length::Fill)
                .style(theme::bg(theme::SELECTED))
                .into(),
        )
    }
}
