//! File locks over a real relay server.

use std::time::{Duration, Instant};

use hythe_relay::proto::{b64, LockInfo};
use hythe_relay::{Client, ClientMsg, Event, ServerMsg, SharedDoc};

const WAIT: Duration = Duration::from_secs(10);

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

fn join(url: &str, name: &str, login: Option<&str>) -> (Client, u64) {
    let client = Client::connect_as(url, "room", name, 0x336699, login.map(String::from));
    let Event::Connected { you, .. } = wait_for(&client, |e| matches!(e, Event::Connected { .. })) else { unreachable!() };
    (client, you)
}

/// The next `Locks` message satisfying `want`.
fn locks_of(client: &Client, mut want: impl FnMut(&[LockInfo]) -> bool) -> Vec<LockInfo> {
    match wait_for(client, |e| matches!(e, Event::Message(ServerMsg::Locks { locks }) if want(locks))) {
        Event::Message(ServerMsg::Locks { locks }) => locks,
        _ => unreachable!(),
    }
}

#[test]
fn first_lock_wins_and_everyone_sees_the_owner() {
    let url = start_server();
    let (a, a_id) = join(&url, "Ada", Some("ada-gh"));
    let (b, _) = join(&url, "Bob", None);
    locks_of(&b, |l| l.is_empty()); // sent right after Welcome
    a.send(ClientMsg::Lock { doc: "a.rs".into() });
    let l = locks_of(&b, |l| l.len() == 1);
    assert_eq!((l[0].doc.as_str(), l[0].owner_id, l[0].owner_name.as_str(), l[0].login.as_deref()), ("a.rs", a_id, "Ada", Some("ada-gh")));
    locks_of(&a, |l| l.len() == 1);

    b.send(ClientMsg::Lock { doc: "a.rs".into() });
    let Event::Message(ServerMsg::LockDenied { doc, owner_name }) =
        wait_for(&b, |e| matches!(e, Event::Message(ServerMsg::LockDenied { .. })))
    else {
        unreachable!()
    };
    assert_eq!((doc.as_str(), owner_name.as_str()), ("a.rs", "Ada"));

    // A late joiner learns of existing locks straight away.
    let (c, _) = join(&url, "Cy", None);
    assert_eq!(locks_of(&c, |_| true).len(), 1);
}

#[test]
fn only_the_owner_can_unlock() {
    let url = start_server();
    let (a, _) = join(&url, "Ada", None);
    let (b, _) = join(&url, "Bob", None);
    a.send(ClientMsg::Lock { doc: "a.rs".into() });
    locks_of(&b, |l| l.len() == 1);
    locks_of(&a, |l| l.len() == 1);
    b.send(ClientMsg::Unlock { doc: "a.rs".into() });
    assert!(no_event_soon(&a), "a stranger's unlock must change nothing");
    a.send(ClientMsg::Unlock { doc: "a.rs".into() });
    locks_of(&b, |l| l.is_empty());
    b.send(ClientMsg::Lock { doc: "a.rs".into() });
    let l = locks_of(&a, |l| l.len() == 1);
    assert_eq!(l[0].owner_name, "Bob");
}

#[test]
fn locks_release_when_the_owner_disconnects() {
    let url = start_server();
    let (a, _) = join(&url, "Ada", None);
    let (b, _) = join(&url, "Bob", None);
    a.send(ClientMsg::Lock { doc: "a.rs".into() });
    locks_of(&b, |l| l.len() == 1);
    drop(a);
    locks_of(&b, |l| l.is_empty());
}

#[test]
fn updates_to_a_file_locked_by_someone_else_are_rejected() {
    let url = start_server();
    let (a, _) = join(&url, "Ada", None);
    let (b, _) = join(&url, "Bob", None);
    let mut doc_a = SharedDoc::new();
    let seed = doc_a.replace(0, 0, "hello");
    a.send(ClientMsg::Open { doc: "a.rs".into() });
    a.send(ClientMsg::Update { doc: "a.rs".into(), data: b64(&seed) });
    wait_for(&b, |e| matches!(e, Event::Message(ServerMsg::Update { .. })));
    a.send(ClientMsg::Lock { doc: "a.rs".into() });
    locks_of(&b, |l| l.len() == 1);
    locks_of(&a, |l| l.len() == 1);

    let mut doc_b = SharedDoc::new();
    doc_b.apply_update(&seed).unwrap();
    let edit = doc_b.replace(5, 0, "!");
    b.send(ClientMsg::Update { doc: "a.rs".into(), data: b64(&edit) });
    wait_for(&b, |e| matches!(e, Event::Message(ServerMsg::LockDenied { .. })));
    assert!(no_event_soon(&a), "the rejected edit must not reach the owner");

    let own = doc_a.replace(5, 0, "?");
    a.send(ClientMsg::Update { doc: "a.rs".into(), data: b64(&own) });
    wait_for(&b, |e| matches!(e, Event::Message(ServerMsg::Update { .. })));
}
