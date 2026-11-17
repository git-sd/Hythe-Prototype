// ============================================================
//  EDIT THIS FILE to change all copy, features, the tech list
//  and where the install button points.
// ============================================================
window.WAITLIST_CONFIG = {
  name: "Hythe",
  title: "Hythe · one thread for your team and your agents",

  // Hero
  headline: "One thread for your team and your agents.",
  sub: "Hythe is a code editor where teammates and AI agents like Claude and Copilot work in the same conversation, right next to the code they're changing.",

  // >>> INSTALL <<<
  // Every install button opens `installUrl`: the newest release, which holds the Windows installer.
  repoUrl: "https://github.com/git-sd/Hythe-Prototype",
  installUrl: "https://github.com/git-sd/Hythe-Prototype/releases/latest",
  releaseApi: "https://api.github.com/repos/git-sd/Hythe-Prototype/releases/latest",
  guideUrl: "https://github.com/git-sd/Hythe-Prototype/blob/main/INSTALL.md",
  installButton: "Download for Windows",
  installTitle: "Install",
  installIntro: "Download the installer and double-click it. No compiling, no Rust, no admin rights. Hythe keeps itself up to date.",
  installNote: "Windows 10 and 11. You need git for source control. Agents are optional: Hythe uses whichever ones you have installed.",
  footer: "© Hythe",

  // >>> AGENTS <<<  The coding-agent CLIs Hythe looks for on your PATH (see src/ai.rs).
  agentsTitle: "Works with the agents you already use",
  agentsText: "Hythe finds the agent command-line tools installed on your computer and runs them with your own accounts. No new subscription, no lock-in to one model.",
  agents: [
    { name: "Claude Code",    cmd: "claude" },
    { name: "GitHub Copilot", cmd: "copilot" },
    { name: "Codex",          cmd: "codex" },
    { name: "Gemini CLI",     cmd: "gemini" },
    { name: "Cursor",         cmd: "cursor-agent" },
    { name: "Aider",          cmd: "aider" },
    { name: "OpenCode",       cmd: "opencode" },
    { name: "Qwen Code",      cmd: "qwen" },
    { name: "Amp",            cmd: "amp" }
  ],

  // >>> FAQ <<<  Edit freely; answers reflect how the prototype works today.
  faqTitle: "Questions",
  faq: [
    { q: "Which AI agents work with Hythe?",
      a: "Any of the nine listed above that you have installed. Hythe runs them under your own account, so you keep the plan and the model you already pay for. Claude Code gets a full conversation view; the others run in a thread terminal." },
    { q: "Where do threads and my code live?",
      a: "In your repository. Threads are saved to git on their own branch, so they travel with the repo and nothing sits in a separate database. Your code stays a normal git repo." },
    { q: "Does live editing go through your servers?",
      a: "No. Live cursors and edits travel through hythe-relay, a small server you run yourself, on your own computer or any machine your team can reach." },
    { q: "Do my teammates all need Hythe?",
      a: "To join a thread or edit live, yes. Anyone else just sees a normal git repository with one extra branch for the threads." },
    { q: "How do I install it?",
      a: "Download Hythe-Setup from the latest GitHub release and double-click it. It installs for your user, adds a Start menu shortcut and updates itself whenever a new version is out. Nothing to compile." },
    { q: "What do I need?",
      a: "Windows 10 or 11 and git. You do not need Rust. Agents are optional: install any you want to use and Hythe finds them." },
    { q: "Is this finished?",
      a: "No. Hythe is a prototype under active development, so expect rough edges and frequent changes. Keep your work committed in git." }
  ],

  // Sections
  demoTitle: "Try it",
  demoText: "A clickable demo of the Hythe window, with scripted teammates and agents. Every button does something.",
  featuresTitle: "What's different",
  featuresText: "Six things that happen in the thread instead of in another tool.",
  stackTitle: "Built on",
  stackText: "Hythe is written in Rust. These are the open-source projects it stands on.",
  ctaTitle: "Try Hythe on your own repo.",

  // >>> FEATURES <<<
  // Each is a thread in the demo's sidebar, a tab on the demo, and a row in "What's different".
  // `id` picks the scene drawn in app.js (threads | comments | diff | live | terminal | history).
  features: [
    { id: "threads",  icon: "chat",    thread: "Team chat",       title: "Shared threads",
      text: "Everyone on the repo and every agent in one conversation. @mention Claude or Copilot and they pick up the work." },
    { id: "comments", icon: "comment", thread: "Retry review",    title: "Comments on words",
      text: "Comment on the exact words of an answer. The phrase stays highlighted and the discussion folds under it." },
    { id: "diff",     icon: "diff",    thread: "Add retries",     title: "Inline diffs",
      text: "Agent edits arrive as diff cards in the thread and in Changes. Accept or reject them where you read them." },
    { id: "live",     icon: "cursor",  thread: "Pair on backoff", title: "Live co-editing",
      text: "Edit the same file together with live cursors. Changes merge as a CRDT, so nobody overwrites anybody." },
    { id: "terminal", icon: "term",    thread: "Run tests",       title: "Terminal",
      text: "A real terminal docks under the editor, and agents can run commands in it. Type a command to try it." },
    { id: "history",  icon: "branch",  thread: "Ship it",         title: "Git built in",
      text: "Fetch, pull, push and commit from the History panel. Threads are stored in git too, so they travel with the repo." }
  ],

  // >>> BUILT ON <<<  Each row opens the project's website in a new tab.
  stack: [
    { name: "Rust",              role: "Language",        url: "https://www.rust-lang.org" },
    { name: "iced",              role: "GPU UI",          url: "https://iced.rs" },
    { name: "Tree-sitter",       role: "Syntax",          url: "https://tree-sitter.github.io/tree-sitter/" },
    { name: "ropey",             role: "Text buffer",     url: "https://github.com/cessen/ropey" },
    { name: "Yrs",               role: "CRDT sync",       url: "https://github.com/y-crdt/y-crdt" },
    { name: "Tokio",             role: "Async runtime",   url: "https://tokio.rs" },
    { name: "tokio-tungstenite", role: "WebSocket relay", url: "https://github.com/snapview/tokio-tungstenite" },
    { name: "Alacritty",         role: "Terminal engine", url: "https://github.com/alacritty/alacritty" },
    { name: "Git",               role: "Thread storage",  url: "https://git-scm.com" },
    { name: "IBM Plex",          role: "Typeface",        url: "https://www.ibm.com/plex/" }
  ]
};
