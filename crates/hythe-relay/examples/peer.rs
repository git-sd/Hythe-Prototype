//! A scripted collaborator, for trying the relay without a second person.
//!
//!     cargo run -p hythe-relay --example peer -- ws://127.0.0.1:7878 room src/main.rs Robin
//!
//! It joins `room`, opens the document, then every couple of seconds adds a
//! comment line near the top and moves its cursor there, printing what it sees.

use std::time::{Duration, Instant};

use hythe_relay::proto::{b64, unb64};
use hythe_relay::{Client, ClientMsg, Event, ServerMsg, SharedDoc};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [url, room, doc_name, name] = match args.as_slice() {
        [a, b, c, d] => [a.clone(), b.clone(), c.clone(), d.clone()],
        _ => {
            eprintln!("usage: peer <ws-url> <room> <document> <name>");
            std::process::exit(2);
        }
    };

    let client = Client::connect(&url, &room, &name, 0xe06c75);
    let mut doc = SharedDoc::new();
    let mut synced = false;
    let mut edits = 0u32;
    let mut next_edit = Instant::now() + Duration::from_secs(2);
    let started = Instant::now();

    println!("[{name}] connecting to {url}, room {room}");
    while started.elapsed() < Duration::from_secs(60) {
        while let Some(event) = client.poll() {
            match event {
                Event::Connected { you, peers } => {
                    println!("[{name}] connected as #{you}; {} other(s) here", peers.len());
                    client.send(ClientMsg::Open { doc: doc_name.clone() });
                }
                Event::Disconnected(why) => {
                    println!("[{name}] disconnected: {why}");
                    return;
                }
                Event::Message(ServerMsg::State { data, seed, .. }) => match data {
                    Some(bytes) => {
                        doc.apply_update(&unb64(&bytes).unwrap()).unwrap();
                        synced = true;
                        println!("[{name}] adopted the shared text ({} bytes)", doc.text().len());
                    }
                    None if seed => {
                        let u = doc.replace(0, 0, "// seeded by the scripted peer\nfn main() {}\n");
                        client.send(ClientMsg::Update { doc: doc_name.clone(), data: b64(&u) });
                        synced = true;
                        println!("[{name}] seeded the document");
                    }
                    None => println!("[{name}] waiting for the seeder"),
                },
                Event::Message(ServerMsg::Update { data, .. }) => {
                    doc.apply_update(&unb64(&data).unwrap()).unwrap();
                    synced = true;
                    println!("[{name}] saw an edit; text is now {} bytes", doc.text().len());
                }
                Event::Message(other) => println!("[{name}] {other:?}"),
            }
        }
        if synced && Instant::now() >= next_edit {
            edits += 1;
            // Insert a line after the first line and park the cursor on it.
            let first_line_end = doc.text().find('\n').map(|i| doc.text()[..i].encode_utf16().count() as u32 + 1).unwrap_or(0);
            let line = format!("// edit {edits} from {name}\n");
            let update = doc.replace(first_line_end, 0, &line);
            client.send(ClientMsg::Update { doc: doc_name.clone(), data: b64(&update) });
            let cursor = first_line_end + line.encode_utf16().count() as u32 - 1;
            client.send(ClientMsg::Cursor { doc: doc_name.clone(), head: cursor, anchor: cursor });
            println!("[{name}] typed edit {edits}");
            next_edit = Instant::now() + Duration::from_secs(3);
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    println!("[{name}] done");
}
