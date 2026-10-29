use std::cell::Cell;

use iced::Size;

use crate::doc::Scroll;

/// Direction panes are laid out in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Axis {
    /// Side by side.
    Horizontal,
    /// Stacked top to bottom.
    Vertical,
}

/// A tab is a view onto a document (index into `App::docs`) with its own scroll.
pub struct Tab {
    pub doc: usize,
    pub scroll: Scroll,
}

pub struct Pane {
    pub tabs: Vec<Tab>,
    pub active: usize,
    /// Size of the editor area, recorded while the pane is drawn.
    pub viewport: Cell<Size>,
    /// Share of the split this pane gets, changed by dragging the divider.
    pub weight: f32,
}

impl Pane {
    fn new() -> Self {
        Self {
            tabs: Vec::new(),
            active: 0,
            viewport: Cell::new(Size::new(600.0, 600.0)),
            weight: 1.0,
        }
    }

    pub fn current(&self) -> Option<&Tab> {
        self.tabs.get(self.active)
    }
}

/// A flat row (or column) of panes; one has focus.
pub struct Panes {
    pub list: Vec<Pane>,
    pub focus: usize,
    pub axis: Axis,
}

impl Panes {
    pub fn new() -> Self {
        Self {
            list: vec![Pane::new()],
            focus: 0,
            axis: Axis::Horizontal,
        }
    }

    pub fn current_doc(&self) -> Option<usize> {
        self.list.get(self.focus)?.current().map(|t| t.doc)
    }

    /// Focused document index, or `usize::MAX` (never a valid index) if none.
    pub fn cur(&self) -> usize {
        self.current_doc().unwrap_or(usize::MAX)
    }

    pub fn focus_pane(&mut self, pane: usize) {
        if pane < self.list.len() {
            self.focus = pane;
        }
    }

    pub fn focus_next(&mut self) {
        self.focus = (self.focus + 1) % self.list.len();
    }

    /// Show `doc` in the focused pane, reusing its tab there if it has one.
    pub fn open_doc(&mut self, doc: usize) {
        let pane = &mut self.list[self.focus];
        match pane.tabs.iter().position(|t| t.doc == doc) {
            Some(i) => pane.active = i,
            None => {
                pane.tabs.push(Tab { doc, scroll: Scroll::default() });
                pane.active = pane.tabs.len() - 1;
            }
        }
    }

    /// Open a new pane after the focused one, showing the same file, and
    /// focus it. Splitting along a different axis re-flows all panes.
    pub fn split(&mut self, axis: Axis) {
        let mut pane = Pane::new();
        if let Some(t) = self.list[self.focus].current() {
            pane.tabs.push(Tab { doc: t.doc, scroll: t.scroll });
        }
        self.axis = axis;
        self.list.insert(self.focus + 1, pane);
        self.focus += 1;
    }

    /// Move a tab to another position, in the same pane or into another one,
    /// and return where it ended up. Tabs only leave a pane that keeps at least
    /// one other tab, and a pane never gets the same document twice.
    pub fn move_tab(&mut self, from: (usize, usize), to: (usize, usize)) -> (usize, usize) {
        let (fp, ft) = from;
        let (tp, tt) = to;
        if fp >= self.list.len() || tp >= self.list.len() || ft >= self.list[fp].tabs.len() {
            return from;
        }
        if fp == tp {
            let pane = &mut self.list[fp];
            let tt = tt.min(pane.tabs.len() - 1);
            if tt == ft {
                return from;
            }
            let tab = pane.tabs.remove(ft);
            pane.tabs.insert(tt, tab);
            pane.active = if pane.active == ft {
                tt
            } else if ft < pane.active && tt >= pane.active {
                pane.active - 1
            } else if ft > pane.active && tt <= pane.active {
                pane.active + 1
            } else {
                pane.active
            };
            return (fp, tt);
        }
        let doc = self.list[fp].tabs[ft].doc;
        if self.list[fp].tabs.len() < 2 || self.list[tp].tabs.iter().any(|t| t.doc == doc) {
            return from;
        }
        let src = &mut self.list[fp];
        let tab = src.tabs.remove(ft);
        src.active = if src.active > ft { src.active - 1 } else { src.active.min(src.tabs.len() - 1) };
        let dst = &mut self.list[tp];
        let at = tt.min(dst.tabs.len());
        dst.tabs.insert(at, tab);
        dst.active = at;
        self.focus = tp;
        (tp, at)
    }

    /// Close one tab. Returns its document so the caller can drop it if no
    /// other tab uses it. A split pane left with no tabs is removed.
    pub fn close_tab(&mut self, pane: usize, tab: usize) -> Option<usize> {
        let p = self.list.get_mut(pane)?;
        if tab >= p.tabs.len() {
            return None;
        }
        let removed = p.tabs.remove(tab);
        if p.active > tab || p.active >= p.tabs.len() {
            p.active = p.active.saturating_sub(1);
        }
        if p.tabs.is_empty() && self.list.len() > 1 {
            self.remove_pane(pane);
        }
        Some(removed.doc)
    }

    /// Close a whole pane (no-op for the last one). Returns the documents it showed.
    pub fn close_pane(&mut self, pane: usize) -> Vec<usize> {
        if self.list.len() < 2 || pane >= self.list.len() {
            return Vec::new();
        }
        let docs = self.list[pane].tabs.iter().map(|t| t.doc).collect();
        self.remove_pane(pane);
        docs
    }

    fn remove_pane(&mut self, i: usize) {
        self.list.remove(i);
        if self.focus > i || self.focus >= self.list.len() {
            self.focus = self.focus.saturating_sub(1);
        }
    }

    /// Number of tabs, across all panes, showing `doc`.
    pub fn refs(&self, doc: usize) -> usize {
        self.list.iter().flat_map(|p| &p.tabs).filter(|t| t.doc == doc).count()
    }

    /// Number of tabs showing `doc` in panes other than `pane`.
    pub fn refs_outside(&self, doc: usize, pane: usize) -> usize {
        (0..self.list.len())
            .filter(|&i| i != pane)
            .map(|i| self.list[i].tabs.iter().filter(|t| t.doc == doc).count())
            .sum()
    }

    pub fn uses_doc(&self, doc: usize) -> bool {
        self.list.iter().any(|p| p.tabs.iter().any(|t| t.doc == doc))
    }

    /// Renumber tabs after document `removed` was deleted from the doc list.
    pub fn forget_doc(&mut self, removed: usize) {
        for tab in self.list.iter_mut().flat_map(|p| p.tabs.iter_mut()) {
            if tab.doc > removed {
                tab.doc -= 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_close_and_renumber() {
        let mut p = Panes::new();
        p.open_doc(0);
        p.open_doc(1);
        p.split(Axis::Vertical);
        assert_eq!(p.list.len(), 2);
        assert_eq!(p.focus, 1);
        assert_eq!(p.current_doc(), Some(1));

        // Closing the only tab in the split pane removes the pane.
        assert_eq!(p.close_tab(1, 0), Some(1));
        assert_eq!(p.list.len(), 1);
        assert_eq!(p.focus, 0);
        assert!(p.uses_doc(1)); // still open in the first pane
        assert_eq!(p.refs(1), 1);
        assert_eq!(p.refs_outside(1, 0), 0);
        assert_eq!(p.refs_outside(1, 1), 1);

        p.close_tab(0, 1);
        assert!(!p.uses_doc(1));
        p.open_doc(2);
        p.forget_doc(1);
        assert_eq!(p.current_doc(), Some(1));
    }

    #[test]
    fn close_pane_keeps_last() {
        let mut p = Panes::new();
        p.open_doc(0);
        assert!(p.close_pane(0).is_empty());
        p.split(Axis::Horizontal);
        assert_eq!(p.close_pane(0), vec![0]);
        assert_eq!(p.list.len(), 1);
    }


    fn panes(tabs: &[&[usize]]) -> Panes {
        let mut p = Panes::new();
        p.list.clear();
        for docs in tabs {
            let mut pane = Pane::new();
            pane.tabs = docs.iter().map(|&doc| Tab { doc, scroll: Scroll::default() }).collect();
            p.list.push(pane);
        }
        p
    }

    fn order(p: &Panes, pane: usize) -> Vec<usize> {
        p.list[pane].tabs.iter().map(|t| t.doc).collect()
    }

    #[test]
    fn tabs_reorder_and_the_active_tab_follows() {
        let mut p = panes(&[&[0, 1, 2, 3]]);
        p.list[0].active = 1;
        assert_eq!(p.move_tab((0, 0), (0, 2)), (0, 2));
        assert_eq!(order(&p, 0), [1, 2, 0, 3]);
        assert_eq!(p.list[0].active, 0, "doc 1 is still the active tab");
        assert_eq!(p.move_tab((0, 0), (0, 3)), (0, 3));
        assert_eq!(p.list[0].active, 3, "the dragged tab was active");
        assert_eq!(p.move_tab((0, 1), (0, 1)), (0, 1));
    }

    #[test]
    fn tabs_move_between_panes_but_never_empty_one() {
        let mut p = panes(&[&[0, 1], &[2], &[1]]);
        assert_eq!(p.move_tab((0, 0), (1, 0)), (1, 0));
        assert_eq!((order(&p, 0), order(&p, 1)), (vec![1], vec![0, 2]));
        assert_eq!(p.focus, 1);
        // Pane 0 is down to one tab: it stays.
        assert_eq!(p.move_tab((0, 0), (1, 0)), (0, 0));
        // Pane 2 already shows doc 1.
        let mut q = panes(&[&[0, 1], &[1]]);
        assert_eq!(q.move_tab((0, 1), (1, 0)), (0, 1));
    }
}
