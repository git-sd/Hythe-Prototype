(function () {
  var C = window.WAITLIST_CONFIG || {};
  var $ = function (id) { return document.getElementById(id); };
  function esc(t) { return String(t).replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;"); }
  var reduce = window.matchMedia && matchMedia("(prefers-reduced-motion: reduce)").matches;
  var mq = function (q) { return window.matchMedia && matchMedia(q).matches; };

  // ---------- copy from config ----------
  document.querySelectorAll("[data-text]").forEach(function (el) {
    var v = C[el.getAttribute("data-text")];
    if (v != null) el.textContent = v;
  });
  document.title = C.title || C.name;
  var md = document.querySelector('meta[name="description"]');
  if (md) md.content = C.sub || "";

  // ---------- install: every install link opens the latest GitHub release ----------
  document.querySelectorAll("[data-install]").forEach(function (l) { l.href = C.installUrl || C.repoUrl; });
  document.querySelectorAll("[data-repo]").forEach(function (l) { l.href = C.repoUrl; });
  document.querySelectorAll("[data-guide]").forEach(function (l) { l.href = C.guideUrl; });

  // ---------- helpers for the demo ----------
  var F = C.features || [];
  var ic = function (n) { return '<i class="ic" data-i="' + n + '"></i>'; };
  var AV = {
    you: '<img class="av u1" src="https://randomuser.me/api/portraits/women/68.jpg" alt="" loading="lazy">',   // Maya, the demo's user
    sam: '<img class="av u2" src="https://randomuser.me/api/portraits/men/32.jpg" alt="" loading="lazy">',
    ada: '<img class="av u3" src="https://randomuser.me/api/portraits/women/44.jpg" alt="" loading="lazy">',
    claude: '<span class="av ag claude">' + ic("sparkle") + "</span>",
    copilot: '<span class="av ag copilot">C</span>'
  };
  var mention = function (t) { return esc(t).replace(/@(\w+)/g, '<span class="at">@$1</span>'); };
  function m(who, html) {
    return '<div class="m">' + AV[who] + '<div class="body">' + html + '</div><span class="mi">' +
      '<button class="ib" data-mact="quote" title="Comment">' + ic("comment") + '</button>' +
      '<button class="ib" data-mact="hide" title="Hide">' + ic("close") + "</button></span></div>";
  }

  // ---------- "What's different" rows + Cargo.toml-style "Built on" list ----------
  $("featList").innerHTML = F.map(function (f, i) {
    return '<div class="fitem">' + ic(f.icon || "sparkle") + "<div><h3>" + esc(f.title) + "</h3><p>" + esc(f.text) +
      '</p></div><button class="open" data-try="' + i + '">Open in demo ' + ic("arrow") + "</button></div>";
  }).join("");
  $("stackList").innerHTML = (C.stack || []).map(function (s) {
    return '<a class="dep" href="' + esc(s.url) + '" target="_blank" rel="noopener noreferrer"><span class="k2">' + esc(s.name) +
      '</span><span class="r">' + esc(s.role) + "</span>" + ic("out").replace('class="ic"', 'class="ic go"') + "</a>";
  }).join("");
  // ---------- agents grid + FAQ ----------
  $("agentList").innerHTML = (C.agents || []).map(function (g) {
    return '<div class="agent"><span class="an">' + esc(g.name) + '</span><code>' + esc(g.cmd) + "</code></div>";
  }).join("");
  $("faqList").innerHTML = (C.faq || []).map(function (f) {
    return "<details><summary>" + esc(f.q) + ic("plus") + "</summary><p>" + esc(f.a) + "</p></details>";
  }).join("");

  if (!F.length) { $("demo").remove(); return; }

  // ---------- code + highlighting ----------
  var KW = /\b(pub|async|fn|let|mut|for|in|match|return|if|use|impl|await|loop|while|mod|struct)\b/g;
  function hl(line) {
    var ci = line.indexOf("//"), com = "";
    if (ci >= 0) { com = line.slice(ci); line = line.slice(0, ci); }
    var out = esc(line)
      .replace(/("[^"]*")/g, "\u0001$1\u0002")
      .replace(/\b(\d[\d_]*)\b/g, '<span class="n">$1</span>')
      .replace(KW, '<span class="k">$1</span>')
      .replace(/\b([A-Z][A-Za-z]+)\b/g, '<span class="ty">$1</span>')
      .replace(/\b([a-z_][a-z0-9_]*)(?=\()/g, '<span class="f">$1</span>')
      .replace(/\u0001/g, '<span class="s">').replace(/\u0002/g, "</span>");
    return out + (com ? '<span class="c">' + esc(com) + "</span>" : "");
  }
  function code(lines) {
    var n = 1;
    return lines.map(function (l) {
      var kind = "", t = l;
      if (Array.isArray(l)) { kind = l[0] === "+" ? "add" : "del"; t = l[1]; }
      return '<div class="ln ' + kind + '"><span class="g">' + (kind === "del" ? "" : n++) + '</span><span class="t">' + hl(t) + "</span></div>";
    }).join("");
  }
  var FILES = {
    "src/sync.rs": [
      "use std::time::Duration;", "",
      "pub async fn sync(client: &Client) -> Result<()> {",
      "    client.push().await?;",
      "    client.pull().await?;",
      "    Ok(())", "}", "",
      "fn backoff(attempt: u32) -> Duration {",
      "    Duration::from_millis(200 * 2u64.pow(attempt))", "}"
    ],
    "tests/sync_test.rs": [
      "use hythe::sync;", "",
      "#[tokio::test]",
      "async fn flaky_server_recovers() {",
      "    let server = FakeServer::flaky(3);",
      "    assert!(sync(&server.client()).await.is_ok());",
      "}", "",
      "#[tokio::test]",
      "async fn dead_server_gives_up() {",
      "    let server = FakeServer::dead();",
      "    assert!(sync(&server.client()).await.is_err());",
      "}"
    ],
    "Cargo.toml": [
      "[package]", "name = \"hythe\"", "version = \"0.1.11\"", "edition = \"2021\"", "",
      "[dependencies]", "iced = \"0.14\"", "ropey = \"1.6\"", "tree-sitter = \"0.27\"", "tokio = \"1\""
    ]
  };
  var DIFF = [
    "use std::time::Duration;", "",
    "pub async fn sync(client: &Client) -> Result<()> {",
    ["-", "    client.push().await?;"],
    ["+", "    for attempt in 0..5 {"],
    ["+", "        match client.push().await {"],
    ["+", "            Ok(_) => break,"],
    ["+", "            Err(e) if e.is_transient() => sleep(backoff(attempt)).await,"],
    ["+", "            Err(e) => return Err(e),"],
    ["+", "        }"],
    ["+", "    }"],
    "    client.pull().await?;", "    Ok(())", "}"
  ];

  var SCENES = {
    threads: {
      focus: "thread", avs: ["you", "sam", "claude", "copilot"],
      feed: [
        m("you", "<p>" + mention("@claude sync drops the connection under load. Add retries with backoff?") + "</p>"),
        m("claude", "<p>On it. I'm reading <code>src/sync.rs</code> and the client config, then I'll post the change here.</p>"),
        m("sam", "<p>" + mention("@copilot can you write the tests while Claude does that?") + "</p>"),
        m("copilot", "<p>Sure. Writing tests for flaky, dead and recovering servers.</p>")
      ]
    },
    comments: {
      focus: "thread", avs: ["you", "sam", "claude"],
      feed: [
        m("you", "<p>" + mention("@claude how long can a sync take now?") + "</p>"),
        m("claude", '<p>Each failed push waits longer than the last, using <button class="mark">exponential backoff</button> from 200ms over five attempts, so about 6s in the worst case.</p>' +
          '<button class="fold open">' + ic("chevr") + '<span class="cnt">1 comment</span></button>' +
          '<div class="quote"><q>“exponential backoff”</q><div class="who">' + AV.sam + "Sam</div>Can we cap each wait at 5s so it never hangs the UI?" +
          '<form class="reply"><input placeholder="Reply…" aria-label="Reply to the comment" maxlength="120"></form></div>'),
        m("claude", "<p>Done. Each wait is now capped at 5s.</p>")
      ]
    },
    diff: {
      focus: "thread", avs: ["you", "claude"], tab: 'Changes <span class="n">1</span>',
      sub: 'Since <span style="color:var(--text)">main</span> <span class="add">+7</span> <span class="del">−1</span>', code: DIFF, add: 7, del: 1,
      feed: [
        m("you", "<p>" + mention("@claude add the retries") + "</p>"),
        m("claude", "<p>Here's the change. Five attempts, transient errors only.</p>" +
          '<div class="card"><div class="ch">' + ic("sparkle") + "Edit <b>src/sync.rs</b>" +
          '<span class="acts"><button class="yes" data-v="ok">Accept</button><button data-v="no">Reject</button></span></div>' +
          '<div class="l del">-    client.push().await?;</div>' +
          '<div class="l add">+    for attempt in 0..5 {</div>' +
          '<div class="l add">+        match client.push().await {</div>' +
          '<div class="l add">+            Ok(_) =&gt; break,</div>' +
          '<div class="l add">+            Err(e) if e.is_transient() =&gt; …</div>' +
          '<div class="l add">+        }</div><div class="l add">+    }</div></div>')
      ]
    },
    live: {
      focus: "editor", avs: ["you", "ada", "sam"], live: true,
      feed: [
        m("ada", "<p>" + mention("@sam I'm in backoff() with you, take the cap?") + "</p>"),
        m("sam", "<p>Yep, adding <code>.min(5_000)</code> now. Click anywhere in the file to join us.</p>")
      ]
    },
    terminal: {
      focus: "editor", avs: ["you", "claude"], term: true, add: 6,
      feed: [
        m("you", "<p>" + mention("@claude run the tests") + "</p>"),
        m("claude", "<p>All 6 tests pass. The terminal below is yours too, try <code>help</code>.</p>")
      ]
    },
    history: {
      focus: "editor", avs: ["you", "sam", "claude"], hist: true, add: 50, del: 1,
      feed: [
        m("claude", "<p>Retries and tests are done. 3 files changed, ready to commit.</p>"),
        m("you", "<p>" + mention("Looks good. Committing from History, @sam push when you're ready.") + "</p>")
      ]
    }
  };

  // ---------- git state for the History panel ----------
  var G = {
    ahead: 4,
    changes: [["M", "sync.rs", "src", 7, 1], ["U", "sync_test.rs", "tests", 42, 0], ["M", "Cargo.toml", "", 1, 0]],
    commits: [
      { t: "v0.1.11: terminal cursor no longer disappears while a shell redraws the line", h: "eacf65d",
        b: "Shells hide the cursor briefly when redrawing. Only full-screen programs may hide it now.", files: [["M", "terminal_view.rs", "src", 12, 3]] },
      { t: "v0.1.10: progress line while agents work, header title fits", h: "00722e3",
        b: "“claude · Thinking… 12s so far” line with a rotating verb, in threads and the team chat.", files: [["M", "ai.rs", "src", 59, 9], ["M", "chat.rs", "src", 5, 1], ["M", "view.rs", "src", 11, 1]] },
      { t: "v0.1.9: fix slowness when agents are listed", h: "724f7f8",
        b: "agents() scanned PATH on every redraw; it is cached now.", files: [["M", "ai.rs", "src", 20, 14]] }
    ],
    total: 74, open: 1
  };
  function frow(f) {
    return '<div class="fr"><span class="st ' + f[0] + '">' + f[0] + "</span>" + esc(f[1]) + (f[2] ? ' <span class="faint sm">' + esc(f[2]) + "</span>" : "") +
      '<span class="grow"></span><span class="add">+' + f[3] + '</span> <span class="del">−' + f[4] + "</span></div>";
  }
  function histHTML() {
    var n = G.changes.length;
    return '<div class="hist">' +
      '<div class="hb">' + ic("branch") + 'main<span class="grow"></span><span class="faint">↑' + G.ahead + " ↓0</span></div>" +
      '<div class="hl"><button class="ib" data-git="fetch">Fetch</button><button class="ib" data-git="pull">Pull</button><button class="ib" data-git="push">Push</button></div>' +
      '<form class="commit"><input class="cmsg" placeholder="Commit message" maxlength="80" aria-label="Commit message"' + (n ? "" : " disabled") + ">" +
      '<button class="cbtn"' + (n ? "" : " disabled") + ">" + (n ? "Commit all " + n + " files" : "Nothing to commit") + "</button></form>" +
      '<div class="hs">Changes · ' + n + "</div>" +
      (n ? G.changes.map(frow).join("") : '<div class="fr faint">No changes</div>') +
      '<div class="hs">History · ' + G.total + "</div>" +
      G.commits.map(function (c, i) {
        return '<div class="cm">' + AV.you + '<button class="cc' + (i === G.open ? " open" : "") + '" data-c="' + i + '"><b>' + esc(c.t) + "</b><span>" + esc(c.b) +
          '</span><span class="hash">' + c.h + '</span><span class="cfiles">' + c.files.map(frow).join("") + "</span></button></div>";
      }).join("") + "</div>";
  }

  // ---------- terminal ----------
  var TERM_START = ['<span class="p">~/hythe</span> main'];
  var term = TERM_START.slice();
  var CMDS = {
    help: "Try: cargo test, cargo build, git status, git log, ls, pwd, whoami, echo, clear",
    ls: "Cargo.toml  assets  crates  src  tests",
    pwd: "/home/maya/hythe",
    whoami: "maya",
    "git status": "On branch main\nChanges not staged for commit:\n  modified:   src/sync.rs\n  modified:   Cargo.toml\nUntracked files:\n  tests/sync_test.rs",
    "cargo build": '<span class="o">   Compiling</span> hythe v0.1.11\n<span class="o">    Finished</span> dev profile in 2.14s',
    "cargo test": '<span class="o">   Compiling</span> hythe v0.1.11\n<span class="o">     Running</span> 6 tests\ntest sync::flaky_server ... <span class="o">ok</span>\ntest sync::dead_server  ... <span class="o">ok</span>\ntest sync::recovers     ... <span class="o">ok</span>\ntest result: <span class="o">ok</span>. 6 passed; 0 failed'
  };
  function runCmd(raw) {
    var c = raw.trim(); if (!c) return;
    term.push("$ " + esc(c));
    if (c === "clear") { term = TERM_START.slice(); return; }
    var key = c.replace(/\s+/g, " ");
    if (/^cargo test/.test(key)) key = "cargo test";
    if (key === "git log") return term.push(G.commits.map(function (x) { return '<span class="p">' + x.h + "</span> " + esc(x.t); }).join("\n"));
    if (/^echo /.test(c)) return term.push(esc(c.slice(5)));
    if (/claude|copilot/i.test(c)) return term.push("Tip: @mention an agent in the thread and it can run commands here.");
    term.push(CMDS[key] || '<span class="e">command not found: ' + esc(c.split(" ")[0]) + "</span>");
  }
  function drawTerm() {
    var t = $("term");
    t.innerHTML = "<pre>" + term.join("\n") + '</pre><form class="tf"><span class="p">$</span><input aria-label="Terminal input" autocomplete="off" spellcheck="false" maxlength="80"></form>';
    t.scrollTop = t.scrollHeight;
  }

  // ---------- state ----------
  var app = $("app"), feed = $("feed"), tabs = $("tabs"), side = $("sideList"), codeEl = $("code"), pop = $("pop");
  var S = { active: null, cur: -1, hist: false, term: false, file: null, ns: false, so: false, nt: false, ne: false, max: null,
            focus: "thread", follow: false, filesOpen: false, extras: [], store: {} };
  var narrow = function () { return mq("(max-width:1000px)"); };
  var phone = function () { return mq("(max-width:680px)"); };

  function toast(text) {
    var t = $("toast"); t.textContent = text; t.classList.add("show");
    clearTimeout(toast.t); toast.t = setTimeout(function () { t.classList.remove("show"); }, 2600);
  }

  function layout() {
    app.classList.toggle("ns", S.ns && !narrow());
    app.classList.toggle("so", S.so && narrow());
    app.classList.toggle("nt", S.nt);
    app.classList.toggle("ne", S.ne);
    app.setAttribute("data-focus", S.focus);
    var threadOn = phone() ? S.focus === "thread" : !S.nt;
    var editorOn = phone() ? S.focus === "editor" : !S.ne;
    var sideOn = narrow() ? S.so : !S.ns;
    var set = function (sel, on) { app.querySelectorAll(sel).forEach(function (b) { b.classList.toggle("on", !!on); }); };
    set('.foot-bar [data-act=history],.side .hdr [data-act=history]', S.hist && editorOn);
    set(".foot-bar [data-act=term]", S.term && !S.hist && editorOn);
    set(".foot-bar [data-act=chat]", threadOn);
    set(".foot-bar [data-act=panel]", editorOn);
    set(".side .hdr [data-act=sidebar]", sideOn);
    set("[data-act=follow]", S.follow);
    set("[data-act=max]", !!S.max);
  }

  function drawEditor() {
    var sc = S.cur >= 0 ? SCENES[F[S.cur].id] || {} : {};
    codeEl.classList.toggle("panel", S.hist);
    if (S.hist) {
      $("eTab").innerHTML = "History"; $("eSub").innerHTML = "";
      codeEl.innerHTML = histHTML();
    } else {
      var file = S.file || (sc.code ? null : "src/sync.rs");
      $("eTab").innerHTML = file ? esc(file.split("/").pop()) : sc.tab;
      $("eSub").innerHTML = file ? "" : sc.sub || "";
      codeEl.innerHTML = code(file ? FILES[file] : sc.code);
      if (sc.live && !S.file) addCursors();
    }
    var t = $("term");
    t.hidden = !S.term || S.hist;
    if (!t.hidden) drawTerm();
    var add = S.cur >= 0 ? (sc.add || 0) : 0, del = S.cur >= 0 ? (sc.del || 0) : 0;
    if (!G.changes.length) { add = 0; del = 0; }
    $("addN").textContent = "+" + add; $("delN").textContent = "−" + del;
    layout();
  }

  // live cursors: Ada and Sam move around; click to drop your own
  var cw = 7.5;
  function measure() {
    var s = document.createElement("span"); s.style.cssText = "position:absolute;visibility:hidden;font:12.5px var(--mono)"; s.textContent = "0000000000";
    codeEl.appendChild(s); cw = s.getBoundingClientRect().width / 10 || 7.5; s.remove();
  }
  function placeCursor(el, line, col) {
    var lns = codeEl.querySelectorAll(".ln"); if (!lns.length) return;
    var ln = lns[Math.max(0, Math.min(line, lns.length - 1))];
    var len = ln.querySelector(".t").textContent.length;
    el.style.top = ln.offsetTop + "px";
    el.style.left = (40 + 12 + Math.min(col, len) * cw - 12) + "px";
  }
  function addCursors() {
    measure();
    codeEl.insertAdjacentHTML("beforeend", '<i class="cur a" data-n="Ada"></i><i class="cur b" data-n="Sam"></i>');
    placeCursor(codeEl.querySelector(".cur.a"), 8, 4);
    placeCursor(codeEl.querySelector(".cur.b"), 9, 44);
  }
  setInterval(function () {
    if (reduce || document.hidden) return;
    var lns = codeEl.querySelectorAll(".ln").length;
    codeEl.querySelectorAll(".cur.a,.cur.b").forEach(function (c) {
      var line = Math.floor(Math.random() * lns);
      var len = codeEl.querySelectorAll(".ln")[line].querySelector(".t").textContent.length;
      placeCursor(c, line, Math.floor(Math.random() * (len + 1)));
    });
  }, 1800);
  codeEl.addEventListener("click", function (e) {
    if (S.hist) return;
    var ln = e.target.closest(".ln"); if (!ln) return;
    var lns = Array.prototype.indexOf.call(codeEl.querySelectorAll(".ln"), ln);
    var t = ln.querySelector(".t"), x = e.clientX - t.getBoundingClientRect().left;
    measure();
    var me = codeEl.querySelector(".cur.me");
    if (!me) { codeEl.insertAdjacentHTML("beforeend", '<i class="cur me" data-n="Maya"></i>'); me = codeEl.querySelector(".cur.me"); }
    placeCursor(me, lns, Math.max(0, Math.round(x / cw)));
  });

  // ---------- sidebar ----------
  function drawSide() {
    var row = function (attrs, cls, inner) { return '<button class="row ' + cls + '" ' + attrs + ">" + inner + "</button>"; };
    var h = '<div class="sh">Projects<span class="grow"></span><button class="ib" data-side="addRepo" title="Add a repository">' + ic("plus") + "</button></div>" +
      row('data-side="website"', "", ic("folder") + "website") +
      row('data-side="hythe"', "", ic("folder") + "hythe<span class=\"grow\"></span>" + ic("more"));
    F.forEach(function (f, i) {
      var on = S.active === "f" + i;
      h += row('data-feat="' + i + '"', "in" + (on ? " on" : ""), ic("sparkle") + esc(f.thread) + (on ? '<span class="busy"></span>' : ""));
    });
    S.extras.forEach(function (x, i) {
      var on = S.active === "x" + i;
      h += row('data-extra="' + i + '"', "in" + (on ? " on" : ""), ic("sparkle") + esc(x.title) + (on ? '<span class="busy"></span>' : ""));
    });
    h += row('data-side="files"', "in dim", ic(S.filesOpen ? "chev" : "chevr") + "Files");
    if (S.filesOpen) Object.keys(FILES).forEach(function (p) {
      h += row('data-file="' + esc(p) + '"', "in2" + (S.file === p && !S.hist ? " on" : ""), ic("file") + esc(p.split("/").pop()) + ' <span class="faint">' + esc(p.indexOf("/") > 0 ? p.split("/")[0] : "") + "</span>");
    });
    h += '<div class="sh">Threads<span class="grow"></span><button class="ib" data-side="newThread" title="New thread">' + ic("plus") + "</button></div>" +
      row('data-side="newTerm"', "", ic("term") + "New terminal") +
      row('data-side="newClaude"', "", ic("sparkle") + "New Claude Code thread") +
      row('data-side="newCopilot"', "", ic("sparkle") + "New GitHub Copilot thread");
    side.innerHTML = h;
  }

  // ---------- switching threads ----------
  function saveFeed() { if (S.active) S.store[S.active] = feed.innerHTML; }
  function setTabs() {
    tabs.querySelectorAll("button").forEach(function (b, j) { var on = j === S.cur; b.classList.toggle("on", on); b.setAttribute("aria-selected", on); });
    var on = tabs.children[S.cur];
    if (on && tabs.scrollWidth > tabs.clientWidth) tabs.scrollTo({ left: on.offsetLeft - (tabs.clientWidth - on.offsetWidth) / 2, behavior: reduce ? "auto" : "smooth" });
  }
  function show(i) {
    saveFeed();
    var f = F[i], s = SCENES[f.id] || SCENES.threads;
    S.active = "f" + i; S.cur = i;
    S.hist = !!s.hist; S.term = !!s.term; S.file = null; S.nt = false; S.ne = false; S.max = null; S.so = false;
    S.focus = s.focus;
    if (s.term) term = TERM_START.concat(["$ cargo test sync", CMDS["cargo test"]]);
    setTabs();
    $("explain").innerHTML = "<b>" + esc(f.title) + ".</b> " + esc(f.text);
    $("tTitle").textContent = f.thread;
    $("tAvs").innerHTML = s.avs.map(function (a) { return AV[a]; }).join("");
    feed.innerHTML = S.store[S.active] || s.feed.map(function (h, k) { return h.replace('class="m"', 'class="m" style="animation-delay:' + (k * 0.1) + 's"'); }).join("");
    drawSide();
    drawEditor();
  }
  function showExtra(i) {
    saveFeed();
    S.active = "x" + i; S.cur = -1; S.focus = "thread"; S.so = false;
    setTabs();
    $("tTitle").textContent = S.extras[i].title;
    $("tAvs").innerHTML = AV.you;
    feed.innerHTML = S.store[S.active] || '<p class="hint">A fresh thread.<br>Say something, or @mention an agent.</p>';
    drawSide(); drawEditor();
  }
  function newThread(prefill) {
    S.extras.push({ title: "New thread " + (S.extras.length + 1) });
    showExtra(S.extras.length - 1);
    var say = $("say"); say.value = prefill || ""; say.focus({ preventScroll: true });
  }

  // ---------- popovers ----------
  var popItems = [];
  function openPop(anchor, items, opts) {
    popItems = items;
    var a = anchor.getBoundingClientRect(), r = app.getBoundingClientRect();
    pop.innerHTML = (opts && opts.search ? '<input id="popq" placeholder="Search threads and features…" aria-label="Search">' : "") + '<div id="popl"></div>';
    pop.hidden = false;
    var drawList = function (q) {
      var list = items.filter(function (it) { return !q || it.label.toLowerCase().indexOf(q) >= 0; });
      $("popl").innerHTML = list.length ? list.map(function (it, k) {
        return '<button data-k="' + items.indexOf(it) + '"' + (k === 0 && q ? ' class="hi"' : "") + ">" + (it.icon ? ic(it.icon) : "") + esc(it.label) + (it.kbd ? "<kbd>" + it.kbd + "</kbd>" : "") + "</button>";
      }).join("") : '<div class="empty">No matches</div>';
    };
    drawList("");
    var w = pop.offsetWidth;
    var left = Math.min(Math.max(8, a.left - r.left), r.width - w - 8);
    pop.style.left = left + "px";
    // open below the button, or above it when there isn't room (the account button sits at the bottom)
    var below = a.bottom - r.top + 6, h = pop.offsetHeight;
    pop.style.top = (below + h > r.height - 6 ? Math.max(6, a.top - r.top - h - 6) : below) + "px";
    if (opts && opts.search) {
      var q = $("popq"); q.focus({ preventScroll: true });
      q.oninput = function () { drawList(q.value.trim().toLowerCase()); };
      q.onkeydown = function (e) {
        if (e.key === "Enter") { var b = $("popl").querySelector("button"); if (b) b.click(); }
      };
    }
  }
  function closePop() { pop.hidden = true; }
  pop.addEventListener("click", function (e) {
    var b = e.target.closest("button[data-k]"); if (!b) return;
    var it = popItems[+b.getAttribute("data-k")]; closePop(); if (it && it.run) it.run();
  });
  document.addEventListener("pointerdown", function (e) { if (!pop.hidden && !pop.contains(e.target) && !e.target.closest("[data-act]")) closePop(); });
  document.addEventListener("keydown", function (e) { if (e.key === "Escape") closePop(); });

  // ---------- actions ----------
  function toggleHistory() {
    S.hist = !S.hist;
    if (S.hist) { S.ne = false; S.file = null; S.focus = "editor"; }
    else if (phone()) S.focus = "thread";
    drawEditor(); drawSide();
  }
  function toggleTerm() {
    if (S.hist) { S.hist = false; S.term = true; }
    else S.term = !S.term;
    if (S.term) { S.ne = false; S.focus = "editor"; }
    drawEditor();
    if (S.term) { var i = $("term").querySelector("input"); if (i) i.focus({ preventScroll: true }); }
  }
  var ACT = {
    sidebar: function () { if (narrow()) S.so = !S.so; else S.ns = !S.ns; layout(); },
    search: function (b) {
      openPop(b, F.map(function (f, i) { return { label: f.thread + " · " + f.title, icon: f.icon, run: function () { stopTour(); show(i); } }; })
        .concat([{ label: "History", icon: "history", run: function () { if (!S.hist) toggleHistory(); } },
                 { label: "Terminal", icon: "term", run: function () { if (!S.term || S.hist) toggleTerm(); } }]), { search: true });
    },
    menu: function (b) {
      openPop(b, [
        { label: "New thread", icon: "plus", kbd: "Ctrl N", run: function () { newThread(); } },
        { label: "New terminal", icon: "term", kbd: "Ctrl `", run: function () { if (!S.term || S.hist) toggleTerm(); } },
        { label: "Search", icon: "search", kbd: "Ctrl P", run: function () { ACT.search(app.querySelector('[data-act=search]')); } },
        { label: (S.hist ? "Hide" : "Show") + " History", icon: "history", run: toggleHistory },
        { label: "Toggle sidebar", icon: "sidebar", kbd: "Ctrl B", run: function () { ACT.sidebar(); } }
      ]);
    },
    history: toggleHistory,
    term: toggleTerm,
    chat: function () {
      if (phone()) S.focus = "thread";
      else { S.nt = !S.nt; if (S.nt && S.ne) S.ne = false; S.max = null; }
      layout();
    },
    panel: function () {
      if (phone()) S.focus = S.focus === "editor" ? "thread" : "editor";
      else { S.ne = !S.ne; if (S.ne && S.nt) S.nt = false; S.max = null; }
      layout();
    },
    max: function () {
      if (phone()) return toast("Already full screen on a phone");
      if (S.max) { S.ns = S.max.ns; S.nt = S.max.nt; S.max = null; }
      else { S.max = { ns: S.ns, nt: S.nt }; S.ns = true; S.nt = true; S.so = false; }
      layout();
    },
    closeEditor: function () {
      if (phone()) { S.focus = "thread"; layout(); return; }
      S.ne = true; S.nt = false; S.max = null; layout();
      toast("Editor closed. The panel button in the footer brings it back.");
    },
    split: function () { toast("Split: open a second file beside this one"); },
    editorMenu: function (b) {
      openPop(b, [
        { label: S.term ? "Close terminal" : "Open terminal", icon: "term", run: toggleTerm },
        { label: (S.hist ? "Hide" : "Show") + " History", icon: "history", run: toggleHistory },
        { label: "Copy path", icon: "file", run: function () { toast("Copied " + (S.file || "src/sync.rs")); } }
      ]);
    },
    follow: function () {
      S.follow = !S.follow; layout();
      toast(S.follow ? "Following Sam. Your view now tracks theirs." : "Stopped following");
    },
    share: function () { toast("Invite ready. Teammates join this thread as full participants."); },
    new: function () { newThread(); },
    threadMenu: function (b) {
      var extra = S.active && S.active[0] === "x";
      openPop(b, [
        { label: "Pin thread", icon: "sparkle", run: function () { toast("Pinned to the top of the sidebar"); } },
        { label: "Copy link", icon: "share", run: function () { toast("Thread link copied"); } },
        { label: "Delete thread", icon: "close", run: function () {
          if (!extra) return toast("The demo threads stay. Make a new one to delete it.");
          var i = +S.active.slice(1); S.extras.splice(i, 1); delete S.store[S.active]; S.active = null; show(0);
        } }
      ]);
    },
    attach: function () { toast("Attach files, images or a terminal selection"); },
    account: function (b) {
      openPop(b, [
        { label: "Settings", icon: "gear", run: function () { toast("Settings live in the real app"); } },
        { label: "Install Hythe", icon: "arrow", run: function () { window.open(C.installUrl || C.repoUrl, "_blank", "noopener"); } }
      ]);
    }
  };
  app.addEventListener("click", function (e) {
    var b = e.target.closest("[data-act]");
    if (b && ACT[b.getAttribute("data-act")]) { e.preventDefault(); stopTour(); ACT[b.getAttribute("data-act")](b); }
  });

  side.addEventListener("click", function (e) {
    var b = e.target.closest("button"); if (!b) return;
    stopTour();
    if (b.hasAttribute("data-feat")) return show(+b.getAttribute("data-feat"));
    if (b.hasAttribute("data-extra")) return showExtra(+b.getAttribute("data-extra"));
    if (b.hasAttribute("data-file")) {
      S.file = b.getAttribute("data-file"); S.hist = false; S.ne = false; S.focus = "editor"; S.so = false;
      drawEditor(); drawSide(); return;
    }
    var a = b.getAttribute("data-side");
    if (a === "files") { S.filesOpen = !S.filesOpen; drawSide(); }
    else if (a === "website") toast("Another repository. Each repo keeps its own threads.");
    else if (a === "hythe") { S.filesOpen = !S.filesOpen; drawSide(); }
    else if (a === "addRepo") toast("Open a folder or clone from GitHub");
    else if (a === "newThread") newThread();
    else if (a === "newTerm") { if (!S.term || S.hist) toggleTerm(); S.so = false; layout(); }
    else if (a === "newClaude") newThread("@claude ");
    else if (a === "newCopilot") newThread("@copilot ");
  });

  // thread feed: comments, replies, diff cards, message buttons
  feed.addEventListener("click", function (e) {
    var mk = e.target.closest(".mark,.fold");
    if (mk) {
      var body = mk.closest(".body"), q = body.querySelector(".quote"), fold = body.querySelector(".fold");
      q.hidden = !q.hidden; fold.classList.toggle("open", !q.hidden);
      if (!q.hidden) { var ri = q.querySelector("input"); if (ri) ri.focus({ preventScroll: true }); }
      return;
    }
    var ab = e.target.closest(".acts button");
    if (ab) {
      var v = ab.getAttribute("data-v");
      ab.parentNode.outerHTML = '<span class="state ' + v + '">' + (v === "ok" ? "✓ Accepted" : "✕ Rejected") + "</span>";
      SCENES.diff.add = v === "ok" ? 7 : 0; SCENES.diff.del = v === "ok" ? 1 : 0;
      $("addN").textContent = "+" + SCENES.diff.add; $("delN").textContent = "−" + SCENES.diff.del;
      toast(v === "ok" ? "Change applied to src/sync.rs" : "Change rejected");
      return;
    }
    var mb = e.target.closest("[data-mact]");
    if (mb) {
      var msgEl = mb.closest(".m");
      if (mb.getAttribute("data-mact") === "hide") { msgEl.remove(); toast("Message hidden"); }
      else { var txt = msgEl.querySelector(".body p").textContent.slice(0, 40); var say = $("say"); say.value = "> " + txt + "… "; say.focus({ preventScroll: true }); }
    }
  });
  feed.addEventListener("submit", function (e) {
    var f = e.target.closest(".reply"); if (!f) return;
    e.preventDefault();
    var inp = f.querySelector("input"), t = inp.value.trim(); if (!t) return;
    f.insertAdjacentHTML("beforebegin", '<div class="who">' + AV.you + "Maya</div>" + esc(t));
    inp.value = "";
    var cnt = f.closest(".body").querySelector(".cnt"), n = (parseInt(cnt.textContent, 10) || 1) + 1;
    cnt.textContent = n + " comments";
  });

  // History panel: fetch / pull / push / commit / open a commit
  codeEl.addEventListener("click", function (e) {
    if (!S.hist) return;
    var g = e.target.closest("[data-git]");
    if (g) {
      var a = g.getAttribute("data-git");
      if (a === "fetch") toast("Fetched origin. Nothing new.");
      else if (a === "pull") toast("Already up to date with origin/main");
      else if (a === "push") { toast(G.ahead ? "Pushed " + G.ahead + " commits to origin/main" : "Nothing to push"); G.ahead = 0; drawEditor(); }
      return;
    }
    var c = e.target.closest(".cc");
    if (c) { var i = +c.getAttribute("data-c"); G.open = G.open === i ? -1 : i; drawEditor(); }
  });
  codeEl.addEventListener("submit", function (e) {
    if (!e.target.closest(".commit")) return;
    e.preventDefault();
    if (!G.changes.length) return;
    var t = e.target.querySelector(".cmsg").value.trim() || "Add retries with backoff to sync";
    G.commits.unshift({ t: t, h: Math.random().toString(16).slice(2, 9), b: "Committed from the demo. Push to share it.", files: G.changes });
    G.changes = []; G.total++; G.ahead++; G.open = 0;
    drawEditor();
    toast("Committed. Push to share it.");
  });

  // terminal input
  $("term").addEventListener("submit", function (e) {
    e.preventDefault();
    var inp = e.target.querySelector("input"); runCmd(inp.value); drawTerm();
    var ni = $("term").querySelector("input"); if (ni) ni.focus({ preventScroll: true });
  });
  $("term").addEventListener("click", function (e) {
    if (e.target.tagName !== "INPUT") { var i = $("term").querySelector("input"); if (i) i.focus({ preventScroll: true }); }
  });

  // composer: type a message, an agent answers
  var REPLIES = {
    claude: ["On it. I'll post the diff in this thread when it's ready.", "Done. The change is in Changes for you to review.", "Good idea. I've added it to the plan for this thread."],
    copilot: ["Sure, I'll take that and report back here.", "Tests are written and passing locally."]
  };
  var said = 0, talking = false;
  $("composer").addEventListener("submit", function (e) {
    e.preventDefault();
    var input = $("say"), t = input.value.trim();
    if (!t || talking) return;
    stopTour();
    input.value = "";
    var hint = feed.querySelector(".hint"); if (hint) hint.remove();
    talking = true;
    feed.insertAdjacentHTML("beforeend", m("you", "<p>" + mention(t) + "</p>"));
    var who = /@copilot/i.test(t) ? "copilot" : "claude";
    feed.insertAdjacentHTML("beforeend", '<div class="m typing-m">' + AV[who] + '<div class="body"><span class="typing"><i></i><i></i><i></i></span></div></div>');
    feed.scrollTop = feed.scrollHeight;
    setTimeout(function () {
      var tp = feed.querySelector(".typing-m"); if (tp) tp.remove();
      var list = REPLIES[who];
      feed.insertAdjacentHTML("beforeend", m(who, "<p>" + esc(list[said++ % list.length]) + "</p>"));
      feed.scrollTop = feed.scrollHeight;
      talking = false;
      saveFeed();
    }, reduce ? 0 : 1100);
  });

  // ---------- tabs, cards, auto tour ----------
  tabs.innerHTML = F.map(function (f, i) {
    return '<button role="tab" data-i="' + i + '">' + ic(f.icon || "sparkle") + esc(f.title) + "</button>";
  }).join("");
  tabs.addEventListener("click", function (e) { var b = e.target.closest("button"); if (b) { stopTour(); show(+b.getAttribute("data-i")); } });
  $("featList").addEventListener("click", function (e) {
    var b = e.target.closest("[data-try]"); if (!b) return;
    stopTour(); show(+b.getAttribute("data-try"));
    $("demo").scrollIntoView({ behavior: reduce ? "auto" : "smooth", block: "start" });
  });

  var auto = null;
  function stopTour() { clearInterval(auto); auto = null; }
  $("demo").addEventListener("pointerdown", stopTour);
  $("demo").addEventListener("keydown", stopTour);
  window.addEventListener("resize", layout);
  show(0);
  if (!reduce) auto = setInterval(function () { if (!document.hidden) show((S.cur + 1) % F.length); }, 8000);
})();
