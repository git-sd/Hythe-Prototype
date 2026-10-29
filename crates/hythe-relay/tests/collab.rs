//! A real relay server and real clients talking over WebSockets.

use std::time::{Duration, Instant};

use hythe_relay::proto::{b64, unb64};
use hythe_relay::{Client, ClientMsg, Event, ServerMsg, SharedDoc};

const WAIT: Duration = Duration::from_secs(10);

/// Start a server on a free port; returns its `ws://` URL.
fn start_server() -> String {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            tx.send(listener.local_addr().unwrap()).unwrap();
            let _ = hythe_relay::server::serve_listener(listener).await;
        });
    });
    format!("ws://{}", rx.recv().unwrap())
}

/// Wait for an event matching `want`, skipping others.
fn wait_for(client: &Client, mut want: impl FnMut(&Event) -> bool) -> Event {
    let deadline = Instant::now() + WAIT;
    loop {
        match client.poll() {
            Some(e) if want(&e) => return e,
            Some(_) => {}
            None if Instant::now() > deadline => panic!("timed out waiting for an event"),
            None => std::thread::sleep(Duration::from_millis(5)),
        }
    }
}

/// Connect and wait for the welcome; returns the client and its id.
fn join(url: &str, room: &str, name: &str) -> (Client, u64) {
    let client = Client::connect(url, room, name, 0x336699);
    let Event::Connected { you, .. } = wait_for(&client, |e| matches!(e, Event::Connected { .. } | Event::Disconnected(_))) else {
        panic!("could not join {room} as {name}")
    };
    (client, you)
}

fn no_event_soon(client: &Client) -> bool {
    let until = Instant::now() + Duration::from_millis(400);
    while Instant::now() < until {
        if client.poll().is_some() {
            return false;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    true
}

fn open_state(client: &Client, doc: &str) -> (Option<Vec<u8>>, bool) {
    client.send(ClientMsg::Open { doc: doc.into() });
    match wait_for(client, |e| matches!(e, Event::Message(ServerMsg::State { .. }))) {
        Event::Message(ServerMsg::State { data, seed, .. }) => (data.map(|d| unb64(&d).unwrap()), seed),
        _ => unreachable!(),
    }
}

#[test]
fn peers_see_each_other_come_and_go() {
    let url = start_server();
    let (a, a_id) = join(&url, "room", "Ada");
    let b = Client::connect(&url, "room", "Bob", 0xaa5500);
    let Event::Connected { you: b_id, peers } = wait_for(&b, |e| matches!(e, Event::Connected { .. })) else { unreachable!() };
    assert_eq!(peers.len(), 1);
    assert_eq!((peers[0].id, peers[0].name.as_str()), (a_id, "Ada"));

    let Event::Message(ServerMsg::PeerJoined { peer }) = wait_for(&a, |e| matches!(e, Event::Message(ServerMsg::PeerJoined { .. }))) else { unreachable!() };
    assert_eq!((peer.id, peer.name.as_str(), peer.color), (b_id, "Bob", 0xaa5500));

    drop(b);
    let Event::Message(ServerMsg::PeerLeft { id }) = wait_for(&a, |e| matches!(e, Event::Message(ServerMsg::PeerLeft { .. }))) else { unreachable!() };
    assert_eq!(id, b_id);
}

#[test]
fn first_opener_seeds_and_later_ones_adopt_the_text() {
    let url = start_server();
    let (a, _) = join(&url, "room", "Ada");
    let (data, seed) = open_state(&a, "src/main.rs");
    assert!(data.is_none() && seed, "the first opener creates the document");

    let mut doc_a = SharedDoc::new();
    let update = doc_a.replace(0, 0, "fn main() {}\n");
    a.send(ClientMsg::Update { doc: "src/main.rs".into(), data: b64(&update) });

    // Bob opens it only after the seed reached the server.
    std::thread::sleep(Duration::from_millis(300));
    let (b, _) = join(&url, "room", "Bob");
    let (data, seed) = open_state(&b, "src/main.rs");
    assert!(!seed);
    let mut doc_b = SharedDoc::new();
    doc_b.apply_update(&data.expect("shared state")).unwrap();
    assert_eq!(doc_b.text(), "fn main() {}\n");
}

#[test]
fn two_simultaneous_openers_never_both_seed() {
    let url = start_server();
    let (a, _) = join(&url, "room", "Ada");
    let (b, _) = join(&url, "room", "Bob");
    let (_, a_seeds) = open_state(&a, "lib.rs");
    let (data, b_seeds) = open_state(&b, "lib.rs");
    assert!(a_seeds && !b_seeds && data.is_none(), "exactly one seeder");

    // The seeder's text reaches the waiting peer as an update.
    let mut doc = SharedDoc::new();
    let update = doc.replace(0, 0, "pub fn x() {}");
    a.send(ClientMsg::Update { doc: "lib.rs".into(), data: b64(&update) });
    let Event::Message(ServerMsg::Update { data, .. }) = wait_for(&b, |e| matches!(e, Event::Message(ServerMsg::Update { .. }))) else { unreachable!() };
    let mut doc_b = SharedDoc::new();
    doc_b.apply_update(&unb64(&data).unwrap()).unwrap();
    assert_eq!(doc_b.text(), "pub fn x() {}");
}

#[test]
fn live_edits_and_concurrent_edits_converge() {
    let url = start_server();
    let (a, _) = join(&url, "room", "Ada");
    let (b, _) = join(&url, "room", "Bob");

    let mut doc_a = SharedDoc::new();
    open_state(&a, "notes.txt");
    let seed = doc_a.replace(0, 0, "hello world");
    a.send(ClientMsg::Update { doc: "notes.txt".into(), data: b64(&seed) });

    let mut doc_b = SharedDoc::new();
    let (state, _) = open_state(&b, "notes.txt");
    match state {
        Some(s) => {
            doc_b.apply_update(&s).unwrap();
        }
        None => {
            let Event::Message(ServerMsg::Update { data, .. }) = wait_for(&b, |e| matches!(e, Event::Message(ServerMsg::Update { .. }))) else { unreachable!() };
            doc_b.apply_update(&unb64(&data).unwrap()).unwrap();
        }
    }
    assert_eq!(doc_b.text(), "hello world");

    // Both type at once without seeing each other's edit.
    let ua = doc_a.replace(5, 0, " big");
    let ub = doc_b.replace(11, 0, "!!");
    a.send(ClientMsg::Update { doc: "notes.txt".into(), data: b64(&ua) });
    b.send(ClientMsg::Update { doc: "notes.txt".into(), data: b64(&ub) });

    let Event::Message(ServerMsg::Update { data, .. }) = wait_for(&b, |e| matches!(e, Event::Message(ServerMsg::Update { .. }))) else { unreachable!() };
    doc_b.apply_update(&unb64(&data).unwrap()).unwrap();
    let Event::Message(ServerMsg::Update { data, .. }) = wait_for(&a, |e| matches!(e, Event::Message(ServerMsg::Update { .. }))) else { unreachable!() };
    doc_a.apply_update(&unb64(&data).unwrap()).unwrap();

    assert_eq!(doc_a.text(), doc_b.text());
    assert_eq!(doc_a.text(), "hello big world!!");
}

#[test]
fn cursors_reach_everyone_but_the_sender() {
    let url = start_server();
    let (a, a_id) = join(&url, "room", "Ada");
    let (b, _) = join(&url, "room", "Bob");
    a.send(ClientMsg::Cursor { doc: "x.rs".into(), head: 12, anchor: 4 });
    let Event::Message(ServerMsg::Cursor { doc, id, head, anchor }) = wait_for(&b, |e| matches!(e, Event::Message(ServerMsg::Cursor { .. }))) else { unreachable!() };
    assert_eq!((doc.as_str(), id, head, anchor), ("x.rs", a_id, 12, 4));
    // Ada hears about Bob joining (already consumed above), but never her own cursor.
    while let Some(e) = a.poll() {
        assert!(!matches!(e, Event::Message(ServerMsg::Cursor { .. })));
    }
}

#[test]
fn rooms_are_isolated() {
    let url = start_server();
    let (a, _) = join(&url, "alpha", "Ada");
    let (outsider, _) = join(&url, "beta", "Eve");
    a.send(ClientMsg::Cursor { doc: "x.rs".into(), head: 1, anchor: 1 });
    let mut doc = SharedDoc::new();
    let update = doc.replace(0, 0, "secret");
    a.send(ClientMsg::Update { doc: "x.rs".into(), data: b64(&update) });
    assert!(no_event_soon(&outsider), "another room must see nothing");
}

#[test]
fn a_document_survives_while_anyone_is_in_the_room() {
    let url = start_server();
    let (a, _) = join(&url, "room", "Ada");
    let (b, _) = join(&url, "room", "Bob");
    open_state(&a, "keep.txt");
    let mut doc = SharedDoc::new();
    let update = doc.replace(0, 0, "kept");
    a.send(ClientMsg::Update { doc: "keep.txt".into(), data: b64(&update) });
    std::thread::sleep(Duration::from_millis(200));
    drop(a);
    // Ada left; Bob (still here) can still fetch the document.
    wait_for(&b, |e| matches!(e, Event::Message(ServerMsg::PeerLeft { .. })));
    let (data, _) = open_state(&b, "keep.txt");
    let mut copy = SharedDoc::new();
    copy.apply_update(&data.unwrap()).unwrap();
    assert_eq!(copy.text(), "kept");
}

#[test]
fn bad_input_is_answered_not_fatal() {
    let url = start_server();
    let (a, _) = join(&url, "room", "Ada");
    a.send(ClientMsg::Update { doc: "x".into(), data: "!!not base64!!".into() });
    wait_for(&a, |e| matches!(e, Event::Message(ServerMsg::Error { .. })));
    // Still usable afterwards.
    let (_, seed) = open_state(&a, "x");
    assert!(seed);
}

#[test]
fn connecting_to_nothing_reports_a_disconnect() {
    // Port 9 (discard) on loopback is almost certainly closed.
    let c = Client::connect("ws://127.0.0.1:9", "room", "Ada", 0);
    let Event::Disconnected(why) = wait_for(&c, |e| matches!(e, Event::Disconnected(_))) else { unreachable!() };
    assert!(!why.is_empty());
}
