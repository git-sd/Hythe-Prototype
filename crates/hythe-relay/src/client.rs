//! A blocking-friendly client: the connection runs on its own thread and the
//! editor polls it, so the UI never waits on the network.

use std::sync::mpsc::{channel, Receiver, TryRecvError};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use tokio::sync::mpsc::{unbounded_channel, UnboundedSender};
use tokio_tungstenite::tungstenite::Message;

use crate::proto::{ClientMsg, Peer, ServerMsg};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// The handshake finished; you are `you`, with `peers` already present.
    Connected { you: u64, peers: Vec<Peer> },
    Message(ServerMsg),
    /// The connection ended or never started.
    Disconnected(String),
}

pub struct Client {
    to_server: UnboundedSender<ClientMsg>,
    events: Receiver<Event>,
}

impl Client {
    /// Start connecting to `url` (for example `ws://127.0.0.1:7878`) and join `room`.
    /// Returns immediately; watch [`Client::poll`] for `Connected` or `Disconnected`.
    pub fn connect(url: &str, room: &str, name: &str, color: u32) -> Client {
        let (to_server, mut commands) = unbounded_channel::<ClientMsg>();
        let (event_tx, events) = channel::<Event>();
        let hello = ClientMsg::Hello { room: room.to_string(), name: name.to_string(), color };
        let url = url.to_string();

        std::thread::spawn(move || {
            let Ok(runtime) = tokio::runtime::Builder::new_current_thread().enable_all().build() else {
                let _ = event_tx.send(Event::Disconnected("could not start the network thread".into()));
                return;
            };
            runtime.block_on(async move {
                let connect = tokio::time::timeout(Duration::from_secs(8), tokio_tungstenite::connect_async(url.as_str()));
                let ws = match connect.await {
                    Ok(Ok((ws, _))) => ws,
                    Ok(Err(e)) => {
                        let _ = event_tx.send(Event::Disconnected(format!("could not connect: {e}")));
                        return;
                    }
                    Err(_) => {
                        let _ = event_tx.send(Event::Disconnected("timed out connecting".into()));
                        return;
                    }
                };
                let (mut sink, mut incoming) = ws.split();
                let Ok(json) = serde_json::to_string(&hello) else { return };
                if sink.send(Message::text(json)).await.is_err() {
                    let _ = event_tx.send(Event::Disconnected("connection closed".into()));
                    return;
                }
                loop {
                    tokio::select! {
                        frame = incoming.next() => match frame {
                            Some(Ok(Message::Text(t))) => {
                                let Ok(msg) = serde_json::from_str::<ServerMsg>(t.as_str()) else { continue };
                                let event = match msg {
                                    ServerMsg::Welcome { you, peers } => Event::Connected { you, peers },
                                    other => Event::Message(other),
                                };
                                if event_tx.send(event).is_err() {
                                    break;
                                }
                            }
                            Some(Ok(Message::Close(_))) | None => {
                                let _ = event_tx.send(Event::Disconnected("the server closed the connection".into()));
                                break;
                            }
                            Some(Err(e)) => {
                                let _ = event_tx.send(Event::Disconnected(e.to_string()));
                                break;
                            }
                            Some(Ok(_)) => {}
                        },
                        command = commands.recv() => match command {
                            Some(msg) => {
                                let Ok(json) = serde_json::to_string(&msg) else { continue };
                                if sink.send(Message::text(json)).await.is_err() {
                                    let _ = event_tx.send(Event::Disconnected("connection lost".into()));
                                    break;
                                }
                            }
                            // The `Client` was dropped: say goodbye and stop.
                            None => {
                                let _ = sink.send(Message::Close(None)).await;
                                break;
                            }
                        },
                    }
                }
            });
        });

        Client { to_server, events }
    }

    pub fn send(&self, msg: ClientMsg) {
        let _ = self.to_server.send(msg);
    }

    /// The next event from the server, if any.
    pub fn poll(&self) -> Option<Event> {
        match self.events.try_recv() {
            Ok(e) => Some(e),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => None,
        }
    }
}
