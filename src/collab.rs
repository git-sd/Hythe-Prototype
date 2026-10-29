//! Live collaboration: connects a repository to a relay room, mirrors edits
//! to and from the shared CRDT, and tracks everyone's cursors.

use std::collections::HashMap;
use std::path::Path;

use hythe_relay::proto::{b64, unb64};
use hythe_relay::{Client, ClientMsg, Event, Peer, ServerMsg, SharedDoc};
use iced::widget::{button, column, container, mouse_area, opaque, row, space, stack, text, text_input};
use iced::{Alignment, Color, Element, Length, Padding, Task, Theme};

use crate::doc::Doc;
use crate::view::{dot, icon_button, BAR_H};
use crate::icons::Icon;
use crate::{theme, App, Message};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Link {
    Connecting,
    Online,
    Offline(String),
}

/// Our side of one shared document.
struct Shared {
    doc: SharedDoc,
    /// We have exchanged state with the room (seeded it, or adopted its text).
    synced: bool,
    last_cursor: Option<(u32, u32)>,
}

pub struct Collab {
    client: Client,
    pub link: Link,
    pub url: String,
    pub room: String,
    pub you: Option<u64>,
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
}

impl Default for CollabUi {
    fn default() -> Self {
        CollabUi {
            open: false,
            url: format!("ws://127.0.0.1:{}", hythe_relay::DEFAULT_PORT),
            room: String::new(),
            name: String::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub enum CollabMsg {
    Panel(bool),
    Url(String),
    Room(String),
    Name(String),
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

fn rgb(c: u32) -> Color {
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
            CollabMsg::Connect => {
                let ui = &self.collab_ui;
                if ui.room.trim().is_empty() || ui.name.trim().is_empty() {
                    self.status = "Enter a room and your name first".into();
                } else {
                    let client = Client::connect(ui.url.trim(), ui.room.trim(), ui.name.trim(), color_for(&ui.name));
                    self.collab = Some(Collab {
                        client,
                        link: Link::Connecting,
                        url: ui.url.trim().to_string(),
                        room: ui.room.trim().to_string(),
                        you: None,
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
            c.docs.insert(id.clone(), Shared { doc: SharedDoc::new(), synced: false, last_cursor: None });
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

    /// Handle everything the server has said since the last tick.
    pub(crate) fn collab_poll(&mut self) {
        let Some(mut c) = self.collab.take() else { return };
        while let Some(event) = c.client.poll() {
            match event {
                Event::Connected { you, peers } => {
                    c.link = Link::Online;
                    c.you = Some(you);
                    c.peers = peers;
                    self.status = format!("Sharing {} on {}", c.room, c.url);
                    self.collab = Some(c);
                    for i in 0..self.docs.len() {
                        self.collab_open_doc(i);
                    }
                    c = self.collab.take().expect("just put back");
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
                if let Some(bytes) = data.as_deref().and_then(|d| unb64(d).ok()) {
                    if shared.doc.apply_update(&bytes).is_ok() {
                        shared.synced = true;
                        let text = shared.doc.text();
                        self.collab_adopt(index, &text);
                    }
                } else if seed {
                    // We're first: our text becomes the shared document.
                    let text = self.docs[index].buffer.text();
                    let update = shared.doc.replace(0, 0, &text);
                    shared.synced = true;
                    c.client.send(ClientMsg::Update { doc, data: b64(&update) });
                    self.docs[index].buffer.track_edits(true);
                }
                // Otherwise someone else is seeding; their update arrives next.
            }
            ServerMsg::Update { doc, data, .. } => {
                let Some(index) = self.collab_doc_index(&doc) else { return };
                let Some(shared) = c.docs.get_mut(&doc) else { return };
                let Ok(bytes) = unb64(&data) else { return };
                let was_synced = shared.synced;
                let Ok(changes) = shared.doc.apply_update(&bytes) else { return };
                shared.synced = true;
                if !was_synced {
                    let text = shared.doc.text();
                    self.collab_adopt(index, &text);
                } else if let Some(d) = self.docs.get_mut(index) {
                    d.buffer.apply_remote(changes.iter().map(|ch| (ch.pos, ch.remove, ch.insert.as_str())));
                    d.dirty = true;
                    d.blame = None;
                    d.rehighlight(&mut self.highlighter);
                }
                self.reveal_cursor();
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
        if c.link != Link::Online {
            return;
        }
        let Some(root) = self.root.as_ref().map(|r| r.path.clone()) else { return };
        let Some(doc) = self.docs.get_mut(self.panes.cur()) else { return };
        let Some(id) = doc_id(&root, doc) else { return };
        let Some(shared) = c.docs.get_mut(&id) else { return };

        let ops = doc.buffer.drain_edits();
        if !shared.synced {
            return; // not part of the room yet; the seed or adoption covers these
        }
        for op in ops {
            let update = shared.doc.replace(op.pos, op.remove, &op.insert);
            c.client.send(ClientMsg::Update { doc: id.clone(), data: b64(&update) });
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

    /// Small coloured circles for the people in the room, for the title bar.
    pub(crate) fn peer_dots(&self) -> Element<'_, Message> {
        let mut r = row![].spacing(4).align_y(Alignment::Center);
        if let Some(c) = &self.collab {
            for p in &c.peers {
                let color = rgb(p.color);
                let initial = p.name.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default();
                r = r.push(
                    container(text(initial).size(11).color(Color::WHITE))
                        .center(Length::Fixed(22.0))
                        .style(move |_: &Theme| container::Style {
                            background: Some(color.into()),
                            border: iced::Border { radius: 11.0.into(), ..Default::default() },
                            ..Default::default()
                        }),
                );
            }
        }
        r.into()
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
                status,
                people,
                action,
                text("Everyone with the same server and room edits together. Run your own server with: hythe-relay --listen 0.0.0.0:7878")
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
