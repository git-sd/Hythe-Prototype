//! `hythe-relay`: run your own collaboration server.
//!
//!     hythe-relay                              listen on 127.0.0.1:7878, in memory
//!     hythe-relay --listen 0.0.0.0:9000        accept connections from other machines
//!     hythe-relay --data-dir ./relay-data      keep documents across restarts
//!     hythe-relay --token SECRET               require a password from every client
//!
//! It does not speak TLS itself. To expose it beyond a trusted network put it
//! behind a reverse proxy that terminates TLS (see the README) and use a
//! `wss://` address in the editor.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use hythe_relay::server::{Relay, ServerConfig};
use hythe_relay::{Auth, DEFAULT_PORT};

const HELP: &str = "usage: hythe-relay [options]

A self-hostable relay for real-time collaboration in Hythe.

  -l, --listen ADDR:PORT     address to listen on (default 127.0.0.1:7878)
      --data-dir DIR         keep document snapshots in DIR and reload them on start
      --token SECRET         require this password from every client
                             (or set HYTHE_RELAY_TOKEN, which keeps it out of `ps`)
      --room-token ROOM=SECRET
                             require SECRET for ROOM only (repeatable; overrides --token)
      --room-ttl SECONDS     keep an empty room in memory this long (default 3600)
      --max-message-mb N     largest message accepted (default 16)
      --max-peers N          peers per room (default 64)
  -h, --help                 show this help";

fn die(msg: &str) -> ! {
    eprintln!("{msg} (try --help)");
    std::process::exit(2);
}

fn main() {
    let runtime = tokio::runtime::Builder::new_multi_thread().enable_all().build().expect("tokio runtime");
    runtime.block_on(run());
}

async fn run() {
    let mut addr = format!("127.0.0.1:{DEFAULT_PORT}");
    let mut cfg = ServerConfig::default();
    let mut server_token = std::env::var("HYTHE_RELAY_TOKEN").ok().filter(|t| !t.is_empty());
    let mut room_tokens: HashMap<String, String> = HashMap::new();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let mut value = |name: &str| args.next().unwrap_or_else(|| die(&format!("{name} needs a value")));
        match arg.as_str() {
            "--listen" | "-l" => addr = value("--listen"),
            "--data-dir" => cfg.data_dir = Some(PathBuf::from(value("--data-dir"))),
            "--token" => server_token = Some(value("--token")),
            "--room-token" => {
                let v = value("--room-token");
                match v.split_once('=') {
                    Some((room, secret)) if !room.is_empty() && !secret.is_empty() => {
                        room_tokens.insert(room.to_string(), secret.to_string());
                    }
                    _ => die("--room-token wants ROOM=SECRET"),
                }
            }
            "--room-ttl" => match value("--room-ttl").parse::<u64>() {
                Ok(s) => cfg.room_ttl = Duration::from_secs(s),
                Err(_) => die("--room-ttl wants a number of seconds"),
            },
            "--max-message-mb" => match value("--max-message-mb").parse::<usize>() {
                Ok(n) if n > 0 => cfg.limits.max_message_bytes = n << 20,
                _ => die("--max-message-mb wants a positive number"),
            },
            "--max-peers" => match value("--max-peers").parse::<usize>() {
                Ok(n) if n > 0 => cfg.limits.max_peers_per_room = n,
                _ => die("--max-peers wants a positive number"),
            },
            "--help" | "-h" => {
                println!("{HELP}");
                return;
            }
            other => die(&format!("unknown argument `{other}`")),
        }
    }
    cfg.auth = Auth { server_token, room_tokens };
    let protected = !cfg.auth.is_open();
    let data_dir = cfg.data_dir.clone();

    let listener = match tokio::net::TcpListener::bind(&addr).await {
        Ok(l) => l,
        Err(e) => {
            eprintln!("cannot listen on {addr}: {e}");
            std::process::exit(1);
        }
    };
    let relay = match Relay::new(cfg) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("cannot use the data directory: {e}");
            std::process::exit(1);
        }
    };
    println!("hythe-relay listening on ws://{addr}");
    match data_dir {
        Some(d) => println!("  saving documents to {}", d.display()),
        None => println!("  documents are kept in memory only (use --data-dir to keep them)"),
    }
    if protected {
        println!("  password required");
    } else if !addr.starts_with("127.") && !addr.starts_with("localhost") {
        eprintln!("  warning: open to the network with no password; consider --token");
    }

    tokio::select! {
        result = relay.serve(listener) => {
            if let Err(e) = result {
                eprintln!("relay stopped: {e}");
                std::process::exit(1);
            }
        }
        _ = tokio::signal::ctrl_c() => {
            println!("shutting down; saving documents");
            relay.flush().await;
        }
    }
}
