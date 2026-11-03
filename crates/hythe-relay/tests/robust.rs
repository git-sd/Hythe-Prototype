//! Persistence, reconnection, auth, seed races and abuse limits, against real
//! servers on loopback.

use std::net::SocketAddr;
use std::time::{Duration, Instant};

use futures_util::SinkExt;
use hythe_relay::proto::{b64, unb64};
use hythe_relay::server::{Relay, ServerConfig};
use hythe_relay::shared::apply_changes;
use hythe_relay::{Auth, Client, ClientMsg, DocSession, Event, Options, Remote, ServerMsg, SharedDoc};
use tokio_tungstenite::tungstenite::Message;

const WAIT: Duration = Duration::from_secs(10);

struct TestServer {
    addr: SocketAddr,
    rt: Option<tokio::runtime::Runtime>,
    relay: Relay,
}

impl TestServer {
    fn start(cfg: ServerConfig) -> TestServer {
        TestServer::start_on("127.0.0.1:0", cfg)
    }

    fn start_on(bind: &str, cfg: ServerConfig) -> TestServer {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let relay = Relay::new(cfg).unwrap();
        let listener = rt.block_on(async {
            for _ in 0..50 {
                match tokio::net::TcpListener::bind(bind).await {
                    Ok(l) => return l,
                    Err(_) => tokio::time::sleep(Duration::from_millis(100)).await,
                }
            }
            panic!("could not bind {bind}")
        });
        let addr = listener.local_addr().unwrap();
        let r = relay.clone();
        rt.spawn(async move {
            let _ = r.serve(listener).await;
        });
        TestServer { addr, rt: Some(rt), relay }
    }

    fn url(&self) -> String {
        format!("ws://{}", self.addr)
    }

    /// Kill the server abruptly: sockets close, memory is gone.
    fn stop(&mut self) {
        if let Some(rt) = self.rt.take() {
            rt.shutdown_timeout(Duration::from_secs(2));
        }
    }

    fn flush(&self) {
        self.rt.as_ref().unwrap().block_on(self.relay.flush());
    }
}

fn tmpdir(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("hythe-relay-test-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

fn fast_cfg() -> ServerConfig {
    ServerConfig { save_interval: Duration::from_millis(50), ..ServerConfig::default() }
}

fn fast_opts() -> Options {
    Options { backoff_min: Duration::from_millis(50), backoff_max: Duration::from_millis(200), ..Options::default() }
}

/// A scripted editor: a Client, a DocSession for one document, and the plain-text copy.
struct Editor {
    client: Client,
    doc: String,
    session: DocSession,
    text: String,
    you: Option<u64>,
    reconnects: u32,
    errors: Vec<String>,
    fatal: Option<String>,
    seeds_received: u32,
    peers_left: u32,
}

impl Editor {
    fn join(url: &str, room: &str, name: &str, doc: &str, text: &str, opts: Options) -> Editor {
        let client = Client::connect_with(url, room, name, 0x112233, opts);
        let e = Editor {
            client,
            doc: doc.into(),
            session: DocSession::new(),
            text: text.into(),
            you: None,
            reconnects: 0,
            errors: vec![],
            fatal: None,
            seeds_received: 0,
            peers_left: 0,
        };
        e.client.send(ClientMsg::Open { doc: doc.into() });
        e
    }

    fn pump(&mut self) {
        while let Some(event) = self.client.poll() {
            match event {
                Event::Connected { you, .. } => self.you = Some(you),
                Event::Reconnecting { .. } => {
                    self.reconnects += 1;
                    self.session.on_disconnect();
                }
                Event::Disconnected(why) => self.fatal = Some(why),
                Event::Message(ServerMsg::State { data, seed, .. }) => {
                    if seed {
                        self.seeds_received += 1;
                    }
                    let bytes = data.map(|d| unb64(&d).unwrap());
                    let o = self.session.on_state(bytes.as_deref(), seed, &self.text).unwrap();
                    self.apply(o);
                }
                Event::Message(ServerMsg::Update { doc, data, .. }) if doc == self.doc => {
                    let o = self.session.on_update(&unb64(&data).unwrap()).unwrap();
                    self.apply(o);
                }
                Event::Message(ServerMsg::Error { message }) => self.errors.push(message),
                Event::Message(ServerMsg::PeerLeft { .. }) => self.peers_left += 1,
                Event::Message(_) => {}
            }
        }
    }

    fn apply(&mut self, o: hythe_relay::Outcome) {
        match o.remote {
            Remote::Nothing => {}
            Remote::Replace(t) => self.text = t,
            Remote::Patch(c) => self.text = apply_changes(&self.text, &c),
        }
        if let Some(u) = o.send {
            self.client.send(ClientMsg::Update { doc: self.doc.clone(), data: b64(&u) });
        }
    }

    fn edit(&mut self, pos: u32, remove: u32, insert: &str) {
        // Mirror into the plain copy exactly like the editor would.
        self.text = apply_changes(&self.text, &[hythe_relay::TextChange { pos, remove, insert: insert.into() }]);
        if let Some(u) = self.session.local_edit(pos, remove, insert) {
            self.client.send(ClientMsg::Update { doc: self.doc.clone(), data: b64(&u) });
        }
    }
}

fn until(editors: &mut [&mut Editor], mut done: impl FnMut(&[&mut Editor]) -> bool, what: &str) {
    let deadline = Instant::now() + WAIT;
    loop {
        for e in editors.iter_mut() {
            e.pump();
        }
        if done(editors) {
            return;
        }
        assert!(Instant::now() < deadline, "timed out waiting for: {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn settle(editors: &mut [&mut Editor], ms: u64) {
    let end = Instant::now() + Duration::from_millis(ms);
    while Instant::now() < end {
        for e in editors.iter_mut() {
            e.pump();
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

// ---------- persistence ----------

#[test]
fn documents_survive_a_server_restart_with_a_data_dir() {
    let dir = tmpdir("restart");
    let cfg = ServerConfig { data_dir: Some(dir.clone()), ..fast_cfg() };
    let mut s1 = TestServer::start(cfg.clone());
    let mut a = Editor::join(&s1.url(), "proj", "Ada", "src/main.rs", "fn main() {}\n", Options { reconnect: false, ..fast_opts() });
    until(&mut [&mut a], |e| e[0].session.synced, "seed");
    a.edit(3, 0, "X");
    settle(&mut [&mut a], 300); // let the saver run
    s1.flush();
    s1.stop();
    drop(a);

    let s2 = TestServer::start(cfg);
    let mut b = Editor::join(&s2.url(), "proj", "Bob", "src/main.rs", "SOMETHING ELSE ON DISK", Options { reconnect: false, ..fast_opts() });
    until(&mut [&mut b], |e| e[0].session.synced, "adopt");
    assert_eq!(b.text, "fn Xmain() {}\n");
    assert_eq!(b.seeds_received, 0, "an existing document is never re-seeded");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn rooms_outlive_their_last_peer_then_expire() {
    let mut cfg = fast_cfg();
    cfg.room_ttl = Duration::from_millis(600);
    let s = TestServer::start(cfg);
    let o = || Options { reconnect: false, ..fast_opts() };
    let mut a = Editor::join(&s.url(), "r", "Ada", "d.txt", "kept text", o());
    until(&mut [&mut a], |e| e[0].session.synced, "seed");
    settle(&mut [&mut a], 100);
    drop(a);
    std::thread::sleep(Duration::from_millis(200));

    let mut b = Editor::join(&s.url(), "r", "Bob", "d.txt", "bob's file", o());
    until(&mut [&mut b], |e| e[0].session.synced, "adopt");
    assert_eq!(b.text, "kept text", "state is kept while the room is empty");
    drop(b);

    // Without a data dir, after the ttl the room is forgotten.
    std::thread::sleep(Duration::from_millis(2500));
    let mut c = Editor::join(&s.url(), "r", "Cy", "d.txt", "cy's file", o());
    until(&mut [&mut c], |e| e[0].session.synced, "seed");
    assert_eq!(c.seeds_received, 1);
    assert_eq!(c.text, "cy's file");
}

#[test]
fn expired_rooms_reload_from_disk() {
    let dir = tmpdir("expire");
    let mut cfg = ServerConfig { data_dir: Some(dir.clone()), ..fast_cfg() };
    cfg.room_ttl = Duration::from_millis(300);
    let s = TestServer::start(cfg);
    let o = || Options { reconnect: false, ..fast_opts() };
    let mut a = Editor::join(&s.url(), "r", "Ada", "d.txt", "on disk", o());
    until(&mut [&mut a], |e| e[0].session.synced, "seed");
    settle(&mut [&mut a], 200);
    drop(a);
    std::thread::sleep(Duration::from_millis(2500)); // evicted from memory
    let mut b = Editor::join(&s.url(), "r", "Bob", "d.txt", "other", o());
    until(&mut [&mut b], |e| e[0].session.synced, "adopt");
    assert_eq!(b.text, "on disk");
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------- reconnect ----------

#[test]
fn clients_reconnect_after_the_server_restarts_and_offline_edits_merge_once() {
    // No persistence at all: the restarted server has forgotten everything.
    let mut s1 = TestServer::start(fast_cfg());
    let bind = s1.addr.to_string();
    let mut a = Editor::join(&s1.url(), "r", "Ada", "n.txt", "hello world", fast_opts());
    until(&mut [&mut a], |e| e[0].session.synced, "A seeds");
    let mut b = Editor::join(&s1.url(), "r", "Bob", "n.txt", "hello world", fast_opts());
    until(&mut [&mut a, &mut b], |e| e[1].session.synced && e[1].text == "hello world", "B adopts");

    s1.stop();
    until(&mut [&mut a, &mut b], |e| e[0].reconnects > 0 && e[1].reconnects > 0, "both notice the drop");
    // Offline edits on both sides.
    a.edit(0, 0, "A:");
    b.edit(11, 0, "!");
    b.edit(11, 0, "?");
    assert_eq!(a.text, "A:hello world");

    let s2 = TestServer::start_on(&bind, fast_cfg());
    let _ = &s2;
    until(
        &mut [&mut a, &mut b],
        |e| e[0].text == "A:hello world?!" || e[0].text == "A:hello world!?" && e[0].text == e[1].text,
        "both converge after reconnecting",
    );
    settle(&mut [&mut a, &mut b], 400);
    assert_eq!(a.text, b.text);
    assert!(a.text.starts_with("A:hello world"), "{:?}", a.text);
    assert_eq!(a.text.matches("hello").count(), 1, "no duplication: {:?}", a.text);
    assert_eq!(a.text.len(), "A:hello world!?".len());
    assert_eq!(a.session.doc.text(), a.text);
    assert_eq!(b.session.doc.text(), b.text);

    // And live editing still works after the reconnect.
    a.edit(0, 0, ">");
    until(&mut [&mut a, &mut b], |e| e[1].text.starts_with(">A:"), "live edit after reconnect");
}

#[test]
fn a_client_that_started_before_the_server_connects_when_it_appears() {
    // Reserve a port, release it, connect (refused), then bring the server up there.
    let probe = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = probe.local_addr().unwrap();
    drop(probe);
    let mut a = Editor::join(&format!("ws://{addr}"), "r", "Ada", "n.txt", "text", fast_opts());
    until(&mut [&mut a], |e| e[0].reconnects >= 2, "a couple of refused attempts");
    let _s = TestServer::start_on(&addr.to_string(), fast_cfg());
    until(&mut [&mut a], |e| e[0].session.synced, "connected and seeded once the server is up");
}

#[test]
fn refused_connections_explain_themselves() {
    let probe = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = probe.local_addr().unwrap();
    drop(probe);
    let c = Client::connect_with(&format!("ws://{addr}"), "r", "n", 0, fast_opts());
    let deadline = Instant::now() + WAIT;
    loop {
        if let Some(Event::Reconnecting { attempt, reason, .. }) = c.poll() {
            assert_eq!(attempt, 1);
            assert!(reason.contains("nothing is listening"), "{reason}");
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
}

// ---------- auth ----------

fn first_terminal(c: &Client) -> Event {
    let deadline = Instant::now() + WAIT;
    loop {
        match c.poll() {
            Some(e @ (Event::Connected { .. } | Event::Disconnected(_))) => return e,
            _ => {}
        }
        assert!(Instant::now() < deadline, "no result");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn a_token_protects_the_server_and_open_servers_ignore_tokens() {
    let cfg = ServerConfig { auth: Auth { server_token: Some("hunter2".into()), ..Auth::default() }, ..fast_cfg() };
    let s = TestServer::start(cfg);

    let c = Client::connect_with(&s.url(), "r", "n", 0, Options { reconnect: true, ..fast_opts() });
    let Event::Disconnected(why) = first_terminal(&c) else { panic!("should be refused") };
    assert!(why.contains("password"), "{why}");

    let c = Client::connect_with(&s.url(), "r", "n", 0, Options { token: Some("nope".into()), ..fast_opts() });
    let Event::Disconnected(why) = first_terminal(&c) else { panic!("should be refused") };
    assert!(why.contains("wrong password"), "{why}");

    let c = Client::connect_with(&s.url(), "r", "n", 0, Options { token: Some("hunter2".into()), ..fast_opts() });
    assert!(matches!(first_terminal(&c), Event::Connected { .. }));

    // A server without a token accepts clients that send one, and ones that don't.
    let open = TestServer::start(fast_cfg());
    let c = Client::connect_with(&open.url(), "r", "n", 0, Options { token: Some("whatever".into()), ..fast_opts() });
    assert!(matches!(first_terminal(&c), Event::Connected { .. }));
    let c = Client::connect(&open.url(), "r", "n", 0);
    assert!(matches!(first_terminal(&c), Event::Connected { .. }));
}

#[test]
fn room_tokens_apply_to_their_room_only() {
    let mut auth = Auth::default();
    auth.room_tokens.insert("private".into(), "p".into());
    let s = TestServer::start(ServerConfig { auth, ..fast_cfg() });
    let c = Client::connect(&s.url(), "private", "n", 0);
    assert!(matches!(first_terminal(&c), Event::Disconnected(_)));
    let c = Client::connect_with(&s.url(), "private", "n", 0, Options { token: Some("p".into()), ..fast_opts() });
    assert!(matches!(first_terminal(&c), Event::Connected { .. }));
    let c = Client::connect(&s.url(), "public", "n", 0);
    assert!(matches!(first_terminal(&c), Event::Connected { .. }));
}

// ---------- seeding ----------

#[test]
fn many_simultaneous_openers_produce_exactly_one_seeder_and_one_copy_of_the_text() {
    let s = TestServer::start(fast_cfg());
    let o = || Options { reconnect: false, ..fast_opts() };
    let mut editors: Vec<Editor> = (0..8).map(|i| Editor::join(&s.url(), "r", &format!("u{i}"), "same.rs", "fn main() {}\n", o())).collect();
    let deadline = Instant::now() + WAIT;
    while !editors.iter().all(|e| e.session.synced) {
        editors.iter_mut().for_each(Editor::pump);
        assert!(Instant::now() < deadline, "not everyone synced");
        std::thread::sleep(Duration::from_millis(10));
    }
    let seeders: u32 = editors.iter().map(|e| e.seeds_received).sum();
    assert_eq!(seeders, 1);
    for e in &editors {
        assert_eq!(e.text, "fn main() {}\n");
        assert_eq!(e.session.doc.text(), "fn main() {}\n");
    }
}

#[test]
fn a_seeder_that_disconnects_hands_the_claim_to_a_waiter() {
    let s = TestServer::start(fast_cfg());
    let o = || Options { reconnect: false, ..fast_opts() };
    // Ada claims the document via a raw client that never sends the seed.
    let ada = Client::connect(&s.url(), "r", "Ada", 0);
    assert!(matches!(first_terminal(&ada), Event::Connected { .. }));
    ada.send(ClientMsg::Open { doc: "d".into() });
    std::thread::sleep(Duration::from_millis(150));

    let mut bob = Editor::join(&s.url(), "r", "Bob", "d", "bob's text", o());
    settle(&mut [&mut bob], 300);
    assert!(!bob.session.synced && bob.seeds_received == 0, "Bob waits for Ada");

    drop(ada);
    until(&mut [&mut bob], |e| e[0].session.synced, "Bob is promoted to seeder");
    assert_eq!(bob.seeds_received, 1);

    let mut cy = Editor::join(&s.url(), "r", "Cy", "d", "cy", o());
    until(&mut [&mut cy], |e| e[0].session.synced, "Cy adopts");
    assert_eq!(cy.text, "bob's text");
}

#[test]
fn seeding_an_empty_file_releases_waiters_and_later_openers() {
    let s = TestServer::start(fast_cfg());
    let o = || Options { reconnect: false, ..fast_opts() };
    let mut a = Editor::join(&s.url(), "r", "Ada", "empty", "", o());
    until(&mut [&mut a], |e| e[0].session.synced, "A seeds an empty doc");
    let mut b = Editor::join(&s.url(), "r", "Bob", "empty", "stale", o());
    until(&mut [&mut b], |e| e[0].session.synced, "B adopts the empty doc");
    assert_eq!(b.text, "");
    assert_eq!(b.seeds_received, 0);
}

// ---------- liveness and limits ----------

#[test]
fn silent_connections_are_dropped_and_peers_told() {
    let cfg = ServerConfig { ping_interval: Duration::from_millis(100), idle_timeout: Duration::from_millis(500), ..fast_cfg() };
    let s = TestServer::start(cfg);
    let mut a = Editor::join(&s.url(), "r", "Ada", "d", "x", Options { reconnect: false, ..fast_opts() });
    until(&mut [&mut a], |e| e[0].you.is_some(), "A joined");

    // A zombie: completes the handshake and says hello, then never reads or writes again.
    let rt = tokio::runtime::Runtime::new().unwrap();
    let _zombie = rt.block_on(async {
        let (mut ws, _) = tokio_tungstenite::connect_async(s.url()).await.unwrap();
        ws.send(Message::text(r#"{"t":"hello","room":"r","name":"Zed","color":1}"#)).await.unwrap();
        ws
    });
    // Ada sees Zed join and then, within the idle timeout, leave.
    let saw_join = |a: &mut Editor| {
        let mut joined = false;
        let mut left = false;
        let deadline = Instant::now() + WAIT;
        while !left && Instant::now() < deadline {
            while let Some(ev) = a.client.poll() {
                match ev {
                    Event::Message(ServerMsg::PeerJoined { peer }) if peer.name == "Zed" => joined = true,
                    Event::Message(ServerMsg::PeerLeft { .. }) => left = true,
                    _ => {}
                }
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        (joined, left)
    };
    assert_eq!(saw_join(&mut a), (true, true));
}

#[test]
fn oversized_messages_disconnect_only_the_sender() {
    let mut cfg = fast_cfg();
    cfg.limits.max_message_bytes = 64 * 1024;
    let s = TestServer::start(cfg);
    let mut good = Editor::join(&s.url(), "r", "Good", "d", "text", Options { reconnect: false, ..fast_opts() });
    let bad = Client::connect(&s.url(), "r", "Bad", 0);
    assert!(matches!(first_terminal(&bad), Event::Connected { .. }));
    until(&mut [&mut good], |e| e[0].session.synced, "good synced");
    bad.send(ClientMsg::Update { doc: "d".into(), data: "A".repeat(200 * 1024) });
    let Event::Disconnected(_) = (loop {
        match bad.poll() {
            Some(e @ Event::Disconnected(_)) => break e,
            _ => std::thread::sleep(Duration::from_millis(10)),
        }
    }) else { unreachable!() };
    // The good client is fine and still connected.
    good.edit(0, 0, "ok ");
    settle(&mut [&mut good], 200);
    assert!(good.fatal.is_none());
}

#[test]
fn flooding_is_cut_off() {
    let mut cfg = fast_cfg();
    cfg.limits.max_msgs_per_sec = 20;
    let s = TestServer::start(cfg);
    let flooder = Client::connect(&s.url(), "r", "Flood", 0);
    assert!(matches!(first_terminal(&flooder), Event::Connected { .. }));
    for i in 0..500 {
        flooder.send(ClientMsg::Cursor { doc: "d".into(), head: i, anchor: i });
    }
    let deadline = Instant::now() + WAIT;
    let mut told = false;
    loop {
        match flooder.poll() {
            Some(Event::Message(ServerMsg::Error { message })) => told |= message.contains("too fast"),
            Some(Event::Disconnected(_)) => break,
            _ => std::thread::sleep(Duration::from_millis(5)),
        }
        assert!(Instant::now() < deadline, "flooder was never cut off");
    }
    assert!(told, "the flooder should be told why");
}

#[test]
fn a_client_that_stops_reading_is_dropped_without_stalling_others() {
    let mut cfg = fast_cfg();
    cfg.limits.max_queue = 8;
    cfg.limits.max_msgs_per_sec = 10_000;
    let s = TestServer::start(cfg);
    let mut a = Editor::join(&s.url(), "r", "Ada", "d", "x", Options { reconnect: false, ..fast_opts() });
    let mut b = Editor::join(&s.url(), "r", "Bob", "d", "x", Options { reconnect: false, ..fast_opts() });
    until(&mut [&mut a, &mut b], |e| e[0].session.synced && e[1].session.synced, "synced");

    // A stuck reader.
    let rt = tokio::runtime::Runtime::new().unwrap();
    let stuck = rt.block_on(async {
        let (mut ws, _) = tokio_tungstenite::connect_async(s.url()).await.unwrap();
        ws.send(Message::text(r#"{"t":"hello","room":"r","name":"Stuck","color":1}"#)).await.unwrap();
        ws
    });
    settle(&mut [&mut a, &mut b], 200);
    // Valid ~130KB updates to a side document, with both live peers draining as they go.
    let mut junk = SharedDoc::new();
    let filler = "x".repeat(100 * 1024);
    for _ in 0..150 {
        let at = junk.len();
        let u = junk.replace(at, 0, &filler);
        a.client.send(ClientMsg::Update { doc: "junk".into(), data: b64(&u) });
        a.pump();
        b.pump();
        std::thread::sleep(Duration::from_millis(2));
    }
    // The stuck peer was cut off (both live peers were told), and Bob still gets live edits.
    until(&mut [&mut a, &mut b], |e| e[0].peers_left >= 1 && e[1].peers_left >= 1, "the stuck peer is dropped");
    a.edit(0, 0, "live ");
    until(&mut [&mut a, &mut b], |e| e[1].text.starts_with("live "), "Bob still receives edits");
    assert!(a.fatal.is_none() && b.fatal.is_none());
    drop(stuck);
    drop(rt);
}

// ---------- TLS plumbing ----------

#[test]
fn wss_urls_are_accepted_and_fail_cleanly_against_a_plain_server() {
    let s = TestServer::start(fast_cfg());
    let c = Client::connect(&format!("wss://{}", s.addr), "r", "n", 0);
    let deadline = Instant::now() + WAIT;
    loop {
        match c.poll() {
            Some(Event::Disconnected(why)) => {
                assert!(!why.is_empty());
                break;
            }
            Some(Event::Connected { .. }) => panic!("plain server cannot speak TLS"),
            _ => std::thread::sleep(Duration::from_millis(10)),
        }
        assert!(Instant::now() < deadline);
    }
}

#[test]
fn shared_doc_type_still_available() {
    // Compile-time check that the public API this suite relies on stays exported.
    let _ = SharedDoc::new();
}
