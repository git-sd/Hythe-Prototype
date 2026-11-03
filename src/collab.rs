//! Live collaboration: connects a repository to a relay room, mirrors edits
//! to and from the shared CRDT, and tracks everyone's cursors.

use std::collections::HashMap;
use std::path::Path;
use std::time::Instant;

use hythe_relay::proto::{b64, unb64};
use hythe_relay::{normalize_url, Client, ClientMsg, DocSession, Event, Options, Peer, Remote, ServerMsg};
use iced::widget::{button, column, container, mouse_area, opaque, row, space, stack, text, text_input};
use iced::{Alignment, Color, Element, Length, Padding, Task, Theme};

use crate::doc::Doc;
use crate::view::{dot, icon_button, BAR_H};
use crate::icons::Icon;
use crate::{theme, App, Message};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Link {
    /// First attempt in progress.
    Connecting,
    Online,
    /// The link is down and the client is retrying. `was_online` says whether
    /// we ever got in (a lost connection) or never reached the server at all.
    Reconnecting { attempt: u32, reason: String, retry_at: Instant, was_online: bool },
    /// Gave up: refused (wrong password) or otherwise not worth retrying.
    Offline(String),
}

/// Our side of one shared document.
struct Shared {
    session: DocSession,
    last_cursor: Option<(u32, u32)>,
}

pub struct Collab {
    client: Client,
    pub link: Link,
    pub url: String,
    pub room: String,
    pub you: Option<u64>,
    /// Joined at least once; distinguishes "lost the connection" from "never reached it".
    was_online: bool,
    pub peers: Vec<Peer>,
    docs: HashMap<String, Shared>,
    cursors: HashMap<(String, u64), (u32, u32)>,
}

/// A collaborator's caret, in editor terms.
pub struct RemoteCursor {
    pub name: String,
    pub color: Color,
    pub head: usize,
}

/// Fields of the Collaborate panel.
pub struct CollabUi {
    pub open: bool,
    pub url: String,
    pub room: String,
    pub name: String,
    /// Optional shared password for the relay or room.
    pub token: String,
}

impl Default for CollabUi {
    fn default() -> Self {
        CollabUi {
            open: false,
            url: format!("ws://127.0.0.1:{}", hythe_relay::DEFAULT_PORT),
            room: String::new(),
            name: String::new(),
            token: String::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub enum CollabMsg {
    Panel(bool),
    Url(String),
    Room(String),
    Name(String),
    Token(String),
    Connect,
    Disconnect,
    Tick,
}

/// A stable 0xRRGGBB colour for a person.
pub fn color_for(name: &str) -> u32 {
    const COLORS: [u32; 7] = [0x61afef, 0xe5c07b, 0x98c379, 0xe06c75, 0xc678dd, 0x56b6c2, 0xd19a66];
    let hash = name.bytes().fold(5u32, |h, b| h.wrapping_mul(33).wrapping_add(b as u32));
    COLORS[hash as usize % COLORS.len()]
}

pub(crate) fn rgb(c: u32) -> Color {
    Color::from_rgb8((c >> 16) as u8, (c >> 8) as u8, c as u8)
}

/// Project-relative path of a file, used as the document's name in the room.
fn doc_id(root: &Path, doc: &Doc) -> Option<String> {
    let rel = doc.path.as_ref()?.strip_prefix(root).ok()?;
    Some(rel.to_string_lossy().replace('\\', "/"))
}

impl App {
    pub(crate) fn collab_update(&mut self, msg: CollabMsg) -> Task<Message> {
        match msg {
            CollabMsg::Panel(open) => {
                self.collab_ui.open = open;
                if open {
                    if self.collab_ui.room.is_empty() {
                        self.collab_ui.room = self
                            .root
                            .as_ref()
                            .map(|r| r.name.clone())
                            .unwrap_or_else(|| "hythe".into());
                    }
                    if self.collab_ui.name.is_empty() {
                        self.collab_ui.name = self.user.clone();
                    }
                }
            }
            CollabMsg::Url(s) => self.collab_ui.url = s,
            CollabMsg::Room(s) => self.collab_ui.room = s,
            CollabMsg::Name(s) => self.collab_ui.name = s,
            CollabMsg::Token(s) => self.collab_ui.token = s,
            CollabMsg::Connect => {
                let ui = &self.collab_ui;
                if ui.room.trim().is_empty() || ui.name.trim().is_empty() {
                    self.status = "Enter a room and your name first".into();
                } else {
                    let url = normalize_url(&ui.url);
                    let opts = Options { token: Some(ui.token.clone()).filter(|t| !t.is_empty()), ..Options::default() };
                    let client = Client::connect_with(&url, ui.room.trim(), ui.name.trim(), color_for(&ui.name), opts);
                    self.collab = Some(Collab {
                        client,
                        link: Link::Connecting,
                        url,
                        room: ui.room.trim().to_string(),
                        you: None,
                        was_online: false,
                        peers: Vec::new(),
                        docs: HashMap::new(),
                        cursors: HashMap::new(),
                    });
                    self.status = "Connecting…".into();
                }
            }
            CollabMsg::Disconnect => self.collab_stop(),
            CollabMsg::Tick => self.collab_poll(),
        }
        Task::none()
    }

    /// Leave the room. Documents stay open as ordinary local files.
    pub(crate) fn collab_stop(&mut self) {
        if self.collab.take().is_some() {
            for d in &mut self.docs {
                d.buffer.track_edits(false);
            }
            self.status = "Stopped sharing".into();
        }
    }

    fn collab_doc_index(&self, id: &str) -> Option<usize> {
        let root = &self.root.as_ref()?.path;
        self.docs.iter().position(|d| doc_id(root, d).as_deref() == Some(id))
    }

    /// Ask the room for the shared state of document `index`.
    pub(crate) fn collab_open_doc(&mut self, index: usize) {
        let (Some(root), Some(c)) = (self.root.as_ref(), self.collab.as_mut()) else { return };
        if c.link != Link::Online {
            return;
        }
        let Some(id) = self.docs.get(index).and_then(|d| doc_id(&root.path, d)) else { return };
        if !c.docs.contains_key(&id) {
            c.docs.insert(id.clone(), Shared { session: DocSession::new(), last_cursor: None });
            c.client.send(ClientMsg::Open { doc: id });
        }
    }

    /// Replace a document's text with the room's version and start tracking edits.
    fn collab_adopt(&mut self, index: usize, text: &str) {
        if let Some(d) = self.docs.get_mut(index) {
            d.replace_text(text, &mut self.highlighter);
            d.dirty = true;
            d.blame = None;
            d.buffer.track_edits(true);
        }
    }

    /// Carry out what a sync step decided: update the editor text, send our state.
    fn collab_apply(&mut self, client: &mut Client, id: &str, index: usize, outcome: hythe_relay::Outcome) {
        match outcome.remote {
            Remote::Nothing => {}
            Remote::Replace(text) => self.collab_adopt(index, &text),
            Remote::Patch(changes) => {
                if let Some(d) = self.docs.get_mut(index) {
                    d.buffer.apply_remote(changes.iter().map(|ch| (ch.pos, ch.remove, ch.insert.as_str())));
                    d.dirty = true;
                    d.blame = None;
                    d.rehighlight(&mut self.highlighter);
                }
            }
        }
        if let Some(update) = outcome.send {
            client.send(ClientMsg::Update { doc: id.to_string(), data: b64(&update) });
        }
        if let Some(d) = self.docs.get_mut(index) {
            d.buffer.track_edits(true);
        }
    }

    /// Handle everything the server has said since the last tick.
    pub(crate) fn collab_poll(&mut self) {
        let Some(mut c) = self.collab.take() else { return };
        while let Some(event) = c.client.poll() {
            match event {
                Event::Connected { you, peers } => {
                    let rejoined = c.was_online;
                    c.link = Link::Online;
                    c.was_online = true;
                    c.you = Some(you);
                    c.peers = peers;
                    c.cursors.clear();
                    for shared in c.docs.values_mut() {
                        shared.last_cursor = None;
                    }
                    self.status = if rejoined {
                        format!("Reconnected to {}; syncing your changes", c.url)
                    } else {
                        format!("Sharing {} on {}", c.room, c.url)
                    };
                    self.collab = Some(c);
                    for i in 0..self.docs.len() {
                        self.collab_open_doc(i);
                    }
                    c = self.collab.take().expect("just put back");
                }
                Event::Reconnecting { attempt, retry_in, reason } => {
                    // Keep recording edits: they stay in the CRDT and sync after reconnecting.
                    for shared in c.docs.values_mut() {
                        shared.session.on_disconnect();
                    }
                    c.peers.clear();
                    c.cursors.clear();
                    self.status = if c.was_online {
                        format!("Connection lost, retrying ({reason})")
                    } else {
                        format!("Cannot reach the relay: {reason}")
                    };
                    c.link = Link::Reconnecting { attempt, reason, retry_at: Instant::now() + retry_in, was_online: c.was_online };
                }
                Event::Disconnected(why) => {
                    c.link = Link::Offline(why.clone());
                    self.status = format!("Collaboration ended: {why}");
                    for d in &mut self.docs {
                        d.buffer.track_edits(false);
                    }
                }
                Event::Message(msg) => self.collab_message(&mut c, msg),
            }
        }
        self.collab = Some(c);
    }

    fn collab_message(&mut self, c: &mut Collab, msg: ServerMsg) {
        match msg {
            ServerMsg::PeerJoined { peer } => {
                self.status = format!("{} joined", peer.name);
                c.peers.push(peer);
            }
            ServerMsg::PeerLeft { id } => {
                if let Some(p) = c.peers.iter().find(|p| p.id == id) {
                    self.status = format!("{} left", p.name);
                }
                c.peers.retain(|p| p.id != id);
                c.cursors.retain(|(_, peer), _| *peer != id);
            }
            ServerMsg::State { doc, data, seed } => {
                let Some(index) = self.collab_doc_index(&doc) else { return };
                let Some(shared) = c.docs.get_mut(&doc) else { return };
                let bytes = data.as_deref().and_then(|d| unb64(d).ok());
                let local = if shared.session.synced { String::new() } else { self.docs[index].buffer.text() };
                match shared.session.on_state(bytes.as_deref(), seed, &local) {
                    Ok(outcome) => self.collab_apply(&mut c.client, &doc, index, outcome),
                    Err(e) => self.status = format!("Relay: {e}"),
                }
                // Otherwise someone else is seeding; their update arrives next.
            }
            ServerMsg::Update { doc, data, .. } => {
                let Some(index) = self.collab_doc_index(&doc) else { return };
                let Some(shared) = c.docs.get_mut(&doc) else { return };
                let Ok(bytes) = unb64(&data) else { return };
                match shared.session.on_update(&bytes) {
                    Ok(outcome) => {
                        self.collab_apply(&mut c.client, &doc, index, outcome);
                        self.reveal_cursor();
                    }
                    Err(e) => self.status = format!("Relay: {e}"),
                }
            }
            ServerMsg::Cursor { doc, id, head, anchor } => {
                c.cursors.insert((doc, id), (head, anchor));
            }
            ServerMsg::Error { message } => self.status = format!("Relay: {message}"),
            ServerMsg::Welcome { .. } => {}
        }
    }

    /// Send our edits and cursor for the focused document. Runs after every update.
    pub(crate) fn collab_flush(&mut self) {
        let Some(c) = self.collab.as_mut() else { return };
        // While reconnecting, edits still go into the CRDT; they are sent after the resync.
        if !matches!(c.link, Link::Online | Link::Reconnecting { .. }) {
            return;
        }
        let Some(root) = self.root.as_ref().map(|r| r.path.clone()) else { return };
        let Some(doc) = self.docs.get_mut(self.panes.cur()) else { return };
        let Some(id) = doc_id(&root, doc) else { return };
        let Some(shared) = c.docs.get_mut(&id) else { return };

        let ops = doc.buffer.drain_edits();
        for op in ops {
            if let Some(update) = shared.session.local_edit(op.pos, op.remove, &op.insert) {
                c.client.send(ClientMsg::Update { doc: id.clone(), data: b64(&update) });
            }
        }
        if c.link != Link::Online || shared.session.is_resyncing() {
            return;
        }
        let cursor = doc.buffer.cursor_utf16();
        if shared.last_cursor != Some(cursor) {
            shared.last_cursor = Some(cursor);
            c.client.send(ClientMsg::Cursor { doc: id, head: cursor.0, anchor: cursor.1 });
        }
    }

    /// Other people's carets in document `index`.
    pub(crate) fn remote_cursors(&self, index: usize) -> Vec<RemoteCursor> {
        let (Some(c), Some(root), Some(doc)) = (self.collab.as_ref(), self.root.as_ref(), self.docs.get(index)) else {
            return Vec::new();
        };
        let Some(id) = doc_id(&root.path, doc) else { return Vec::new() };
        c.cursors
            .iter()
            .filter(|((d, _), _)| *d == id)
            .filter_map(|((_, peer), (head, _))| {
                let p = c.peers.iter().find(|p| p.id == *peer)?;
                Some(RemoteCursor { name: p.name.clone(), color: rgb(p.color), head: doc.buffer.utf16_to_char(*head) })
            })
            .collect()
    }

    /// The Collaborate panel, anchored above the bottom bar on the right.
    pub(crate) fn collab_panel(&self) -> Element<'_, Message> {
        let ui = &self.collab_ui;
        let msg = |m: CollabMsg| Message::Collab(m);
        let field = |label: &'static str, value: &str, on: fn(String) -> CollabMsg| {
            column![
                text(label).size(11).color(theme::TEXT_DIM),
                text_input("", value).on_input(move |s| Message::Collab(on(s))).padding(7).size(13).style(theme::input),
            ]
            .spacing(3)
        };

        let status: Element<'_, Message> = match self.collab.as_ref().map(|c| &c.link) {
            None => text("Not sharing").size(12).color(theme::TEXT_FAINT).into(),
            Some(Link::Connecting) => text("Connecting…").size(12).color(theme::TEXT_DIM).into(),
            Some(Link::Online) => row![dot(theme::ADDED), text("Sharing live").size(12).color(theme::ADDED)]
                .spacing(6)
                .align_y(Alignment::Center)
                .into(),
            Some(Link::Reconnecting { attempt, reason, retry_at, was_online }) => {
                let secs = retry_at.saturating_duration_since(Instant::now()).as_secs() + 1;
                let (headline, detail) = if *was_online {
                    (
                        format!("Connection lost. Reconnecting (try {attempt}, next in {secs}s)"),
                        "Your edits are kept and will sync when the relay is back.".to_string(),
                    )
                } else {
                    (
                        format!("Can't reach the relay (try {attempt}, next in {secs}s)"),
                        format!("{reason}. Start one with `hythe-relay`, or enter the address your host gave you."),
                    )
                };
                column![
                    row![dot(theme::REMOVED), text(headline).size(12).color(theme::REMOVED)].spacing(6).align_y(Alignment::Center),
                    text(detail).size(11).color(theme::TEXT_DIM),
                ]
                .spacing(3)
                .into()
            }
            Some(Link::Offline(why)) => text(format!("Disconnected: {why}")).size(12).color(theme::REMOVED).into(),
        };

        let mut people = column![].spacing(4);
        if let Some(c) = &self.collab {
            people = people.push(text(format!("In the room · {}", c.peers.len() + 1)).size(11).color(theme::TEXT_DIM));
            people = people.push(
                row![dot(rgb(color_for(&ui.name))), text(format!("{} (you)", ui.name)).size(13)]
                    .spacing(8)
                    .align_y(Alignment::Center),
            );
            for p in &c.peers {
                people = people.push(
                    row![dot(rgb(p.color)), text(p.name.clone()).size(13)].spacing(8).align_y(Alignment::Center),
                );
            }
        }

        let sharing = self.collab.is_some();
        let action = if sharing {
            button(text("Stop sharing").size(13).center().width(Length::Fill))
                .width(Length::Fill)
                .padding([7, 10])
                .style(theme::flat(false))
                .on_press(msg(CollabMsg::Disconnect))
        } else {
            button(text("Start sharing").size(13).center().width(Length::Fill))
                .width(Length::Fill)
                .padding([7, 10])
                .style(|t: &Theme, s| {
                    let mut st = theme::flat(true)(t, s);
                    st.background = Some(Color { a: 0.28, ..theme::ACCENT }.into());
                    st.text_color = theme::TEXT;
                    st
                })
                .on_press(msg(CollabMsg::Connect))
        };

        let panel = container(
            column![
                row![text("Collaborate").size(15), space::horizontal(), icon_button(Icon::Close, theme::TEXT_FAINT, false, msg(CollabMsg::Panel(false)))]
                    .align_y(Alignment::Center),
                field("Relay server", &ui.url, CollabMsg::Url),
                field("Room", &ui.room, CollabMsg::Room),
                field("Your name", &ui.name, CollabMsg::Name),
                column![
                    text("Password (if the relay needs one)").size(11).color(theme::TEXT_DIM),
                    text_input("", &ui.token).on_input(|s| Message::Collab(CollabMsg::Token(s))).secure(true).padding(7).size(13).style(theme::input),
                ]
                .spacing(3),
                status,
                people,
                action,
                text("Everyone with the same server and room edits together. 127.0.0.1 means this computer: run `hythe-relay` here, or enter a shared server (ws://host:7878, or wss://host behind HTTPS). Host your own with: hythe-relay --listen 0.0.0.0:7878 --token SECRET --data-dir ./relay-data")
                    .size(11)
                    .color(theme::TEXT_FAINT),
            ]
            .spacing(10)
            .padding(14),
        )
        .width(Length::Fixed(340.0))
        .style(|_: &Theme| container::Style {
            background: Some(theme::PANEL_BG.into()),
            border: iced::Border { color: theme::SELECTED, width: 1.0, radius: 4.0.into() },
            ..Default::default()
        });

        stack![
            mouse_area(space::horizontal().width(Length::Fill).height(Length::Fill)).on_press(msg(CollabMsg::Panel(false))),
            container(opaque(panel))
                .width(Length::Fill)
                .height(Length::Fill)
                .align_x(Alignment::End)
                .align_y(Alignment::End)
                .padding(Padding { bottom: BAR_H + 6.0, right: 10.0, ..Padding::ZERO }),
        ]
        .into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colours_are_stable_per_name() {
        assert_eq!(color_for("Ada"), color_for("Ada"));
        assert!((0..50).map(|i| color_for(&format!("user{i}"))).collect::<std::collections::HashSet<_>>().len() > 3);
    }

    #[test]
    fn document_ids_are_project_relative_with_forward_slashes() {
        let mut d = Doc::empty();
        d.path = Some(std::path::PathBuf::from("C:\\work\\repo\\src\\main.rs"));
        assert_eq!(doc_id(Path::new("C:\\work\\repo"), &d).as_deref(), Some("src/main.rs"));
        assert_eq!(doc_id(Path::new("C:\\other"), &d), None);
    }
}
