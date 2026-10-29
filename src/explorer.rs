use std::path::{Path, PathBuf};

const HIDDEN: &[&str] = &[".git", "target", "node_modules", ".DS_Store"];

pub struct Node {
    pub path: PathBuf,
    pub name: String,
    pub is_dir: bool,
    pub expanded: bool,
    children: Vec<Node>,
    loaded: bool,
}

impl Node {
    pub fn root(path: PathBuf) -> Self {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string());
        let mut node = Node {
            path,
            name,
            is_dir: true,
            expanded: true,
            children: Vec::new(),
            loaded: false,
        };
        node.load();
        node
    }

    fn load(&mut self) {
        self.loaded = true;
        let Ok(entries) = std::fs::read_dir(&self.path) else {
            return;
        };
        let mut children: Vec<Node> = entries
            .filter_map(Result::ok)
            .filter_map(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                if HIDDEN.contains(&name.as_str()) {
                    return None;
                }
                let is_dir = e.file_type().ok()?.is_dir();
                Some(Node {
                    path: e.path(),
                    name,
                    is_dir,
                    expanded: false,
                    children: Vec::new(),
                    loaded: false,
                })
            })
            .collect();
        children.sort_by(|a, b| {
            b.is_dir
                .cmp(&a.is_dir)
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        self.children = children;
    }

    pub fn toggle(&mut self, target: &Path) -> bool {
        if self.path == target {
            self.expanded = !self.expanded;
            if self.expanded && !self.loaded {
                self.load();
            }
            return true;
        }
        self.children.iter_mut().any(|c| c.is_dir && target.starts_with(&c.path) && c.toggle(target))
    }

    /// Visible rows in display order, as (depth, node), excluding the root.
    pub fn rows<'a>(&'a self, depth: usize, out: &mut Vec<(usize, &'a Node)>) {
        if !self.expanded {
            return;
        }
        for child in &self.children {
            out.push((depth, child));
            if child.is_dir {
                child.rows(depth + 1, out);
            }
        }
    }
}
