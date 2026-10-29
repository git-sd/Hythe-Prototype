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
        // Top-level folders start open, like categories in a channel list.
        for child in node.children.iter_mut().filter(|c| c.is_dir) {
            child.expanded = true;
            child.load();
        }
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

    /// Re-read every folder that has been opened, keeping what is expanded, so
    /// files created or deleted by other programs show up.
    pub fn refresh(&mut self) {
        if !self.loaded {
            return;
        }
        let mut old = std::mem::take(&mut self.children);
        self.load();
        for child in &mut self.children {
            if let Some(i) = old.iter().position(|o| o.path == child.path) {
                let o = old.swap_remove(i);
                if child.is_dir && o.loaded {
                    child.expanded = o.expanded;
                    child.children = o.children;
                    child.loaded = true;
                    child.refresh();
                }
            }
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refresh_picks_up_new_files_and_keeps_expansion() {
        let dir = std::env::temp_dir().join(format!("hythe-tree-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("a.txt"), "a").unwrap();
        let mut root = Node::root(dir.clone());
        let names = |root: &Node| {
            let mut rows = Vec::new();
            root.rows(0, &mut rows);
            rows.iter().map(|(_, n)| n.name.clone()).collect::<Vec<_>>()
        };
        assert_eq!(names(&root), ["sub", "a.txt"]);
        std::fs::write(dir.join("sub").join("b.txt"), "b").unwrap();
        std::fs::write(dir.join("c.txt"), "c").unwrap();
        std::fs::remove_file(dir.join("a.txt")).unwrap();
        root.refresh();
        assert_eq!(names(&root), ["sub", "b.txt", "c.txt"]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
