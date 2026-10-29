//! `hythe-relay`: run your own collaboration server.
//!
//!     hythe-relay                       listen on 127.0.0.1:7878
//!     hythe-relay --listen 0.0.0.0:9000 accept connections from other machines
//!
//! Put it behind a TLS-terminating proxy (for example Caddy or nginx) if you
//! expose it beyond a trusted network; it does not authenticate clients.

use hythe_relay::DEFAULT_PORT;

#[tokio::main]
async fn main() {
    let mut addr = format!("127.0.0.1:{DEFAULT_PORT}");
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--listen" | "-l" => match args.next() {
                Some(a) => addr = a,
                None => {
                    eprintln!("--listen needs an address, e.g. 0.0.0.0:{DEFAULT_PORT}");
                    std::process::exit(2);
                }
            },
            "--help" | "-h" => {
                println!("usage: hythe-relay [--listen ADDRESS:PORT]\n\nA self-hostable relay for real-time collaboration in Hythe.");
                return;
            }
            other => {
                eprintln!("unknown argument `{other}` (try --help)");
                std::process::exit(2);
            }
        }
    }
    println!("hythe-relay listening on ws://{addr}");
    if let Err(e) = hythe_relay::server::serve(&addr).await {
        eprintln!("relay stopped: {e}");
        std::process::exit(1);
    }
}
