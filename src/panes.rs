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
    /// Share of the editor area, relative to the other panes.
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
        for p in &mut self.list {
            p.weight = 1.0;
        }
        self.focus += 1;
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

    /// Move the divider between pane `i` and `i + 1` by `delta` screen pixels.
    pub fn drag_divider(&mut self, i: usize, delta: f32) {
        const MIN_PX: f32 = 120.0;
        if i + 1 >= self.list.len() {
            return;
        }
        let px = |p: &Pane| match self.axis {
            Axis::Horizontal => p.viewport.get().width,
            Axis::Vertical => p.viewport.get().height,
        };
        let (a, b) = (px(&self.list[i]), px(&self.list[i + 1]));
        let (wa, wb) = (self.list[i].weight, self.list[i + 1].weight);
        if a <= 0.0 || b <= 0.0 {
            return;
        }
        // Pixels to weight, using the pair's own scale.
        let scale = (wa + wb) / (a + b);
        let delta = delta.clamp(MIN_PX.min(a) - a, b - MIN_PX.min(b));
        self.list[i].weight = wa + delta * scale;
        self.list[i + 1].weight = wb - delta * scale;
    }

    fn remove_pane(&mut self, i: usize) {
        self.list.remove(i);
        for p in &mut self.list {
            p.weight = 1.0;
        }
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
    fn dragging_a_divider_trades_width_between_neighbours() {
        let mut p = Panes::new();
        p.split(Axis::Horizontal);
        for pane in &p.list {
            pane.viewport.set(Size::new(500.0, 400.0));
        }
        p.drag_divider(0, 100.0);
        assert!(p.list[0].weight > 1.0 && p.list[1].weight < 1.0);
        assert!((p.list[0].weight + p.list[1].weight - 2.0).abs() < 1e-4);
        p.drag_divider(0, 10_000.0);
        assert!(p.list[1].weight > 0.0, "the neighbour keeps a minimum size");
    }

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
}
