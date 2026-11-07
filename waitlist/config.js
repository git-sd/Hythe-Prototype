// ============================================================
//  EDIT THIS FILE to change all copy, features, the tech list
//  and the signup endpoint.
// ============================================================
window.WAITLIST_CONFIG = {
  name: "Hythe",
  tagline: "The code editor where you, your team and your AI agents work in one shared thread.",
  eyebrow: "Early access",
  placeholder: "you@email.com",
  button: "Join the waitlist",
  buttonBusy: "Joining…",
  note: "No spam. One email when it's your turn.",
  successTitle: "You're on the list.",
  successBody: "We'll email you the moment there's room.",
  successAgain: "That email is already on the list.",
  errorInvalid: "That doesn't look like a valid email.",
  errorNetwork: "Something went wrong. Please try again.",
  footer: "© Hythe",

  // >>> SIGNUP ENDPOINT <<<
  // POST JSON { "email": "...", "source": "waitlist" }; any 2xx = success.
  // (Formspree, a Make/Zapier webhook, your own API...) While empty, signups are
  // saved in this browser only (localStorage "waitlist_signups") for previewing.
  endpoint: "",

  // Section headings
  demoTitle: "Try it right here",
  demoText: "This is a working copy of the Hythe window. Click around: every button does something.",
  featuresTitle: "Everything happens in the thread",
  featuresText: "Six things Hythe does that a normal editor doesn't. Each one opens in the demo above.",
  stackTitle: "Built on open source",
  stackText: "Hythe is written in Rust and stands on these projects. Click one to visit it.",
  ctaTitle: "Be first in the thread.",
  ctaText: "Hythe is in private testing. Join the waitlist and we'll let you in as soon as there's room.",

  // >>> FEATURES <<<
  // Each is a thread in the demo's sidebar, a tab above the demo, and a card below it.
  // `id` picks the scene drawn in app.js (threads | comments | diff | live | terminal | history).
  features: [
    { id: "threads",  icon: "chat",    thread: "Team chat",       title: "Shared threads",
      text: "Everyone on the repo and every agent in one conversation. @mention Claude or Copilot and they pick up the work." },
    { id: "comments", icon: "comment", thread: "Retry review",    title: "Comments on words",
      text: "Comment on the exact words of an answer. The phrase stays highlighted and the discussion folds under it." },
    { id: "diff",     icon: "diff",    thread: "Add retries",     title: "Inline diffs",
      text: "Agent edits arrive as diff cards in the thread and in Changes. Accept or reject them right there." },
    { id: "live",     icon: "cursor",  thread: "Pair on backoff", title: "Live co-editing",
      text: "Edit the same file together with live cursors and presence, merged as a CRDT so nobody overwrites anybody." },
    { id: "terminal", icon: "term",    thread: "Run tests",       title: "Terminal",
      text: "A real terminal docks under the editor, and agents can run commands in it. Try typing a command." },
    { id: "history",  icon: "branch",  thread: "Ship it",         title: "Git built in",
      text: "Fetch, pull, push and commit from the History panel. Threads are stored in git too, so history travels with the repo." }
  ],

  // >>> BUILT WITH <<<  Each card opens its website in a new tab.
  stack: [
    { name: "Rust",              role: "Language",        url: "https://www.rust-lang.org",                  desc: "Fast and memory safe, top to bottom." },
    { name: "iced",              role: "GPU UI",          url: "https://iced.rs",                            desc: "The cross-platform GUI library Hythe draws with." },
    { name: "Tree-sitter",       role: "Syntax",          url: "https://tree-sitter.github.io/tree-sitter/", desc: "Incremental parsing for highlighting in 18 languages." },
    { name: "ropey",             role: "Text buffer",     url: "https://github.com/cessen/ropey",            desc: "A rope that keeps big files fast to edit." },
    { name: "Yrs",               role: "CRDT sync",       url: "https://github.com/y-crdt/y-crdt",           desc: "Merges live edits so nobody overwrites anybody." },
    { name: "Tokio",             role: "Async runtime",   url: "https://tokio.rs",                           desc: "Runs agents, git and the relay without blocking." },
    { name: "tokio-tungstenite", role: "WebSocket relay", url: "https://github.com/snapview/tokio-tungstenite", desc: "Carries cursors and edits between teammates." },
    { name: "Alacritty",         role: "Terminal engine", url: "https://github.com/alacritty/alacritty",     desc: "The terminal emulator core behind every shell." },
    { name: "Git",               role: "Thread storage",  url: "https://git-scm.com",                        desc: "Source of truth for code and conversations." },
    { name: "IBM Plex",          role: "Typeface",        url: "https://www.ibm.com/plex/",                  desc: "Plex Sans and Plex Mono, everywhere." }
  ]
};
