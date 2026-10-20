//! Snapshot-based undo history.

#[derive(Debug, Clone)]
pub(crate) struct History {
    snapshots: Vec<String>,
    cursor: usize,
    /// Cursor at the last save; `None` once that state is no longer reachable.
    saved: Option<usize>,
}

impl History {
    pub(crate) fn new(initial: String) -> Self {
        Self {
            snapshots: vec![initial],
            cursor: 0,
            saved: Some(0),
        }
    }

    pub(crate) fn current(&self) -> &str {
        &self.snapshots[self.cursor]
    }

    /// Records a new state, dropping any redo states. Returns false (and records
    /// nothing) when the text is unchanged.
    pub(crate) fn push(&mut self, text: String) -> bool {
        if text == self.current() {
            return false;
        }
        self.snapshots.truncate(self.cursor + 1);
        if self.saved.is_some_and(|s| s > self.cursor) {
            self.saved = None;
        }
        self.snapshots.push(text);
        self.cursor += 1;
        true
    }

    pub(crate) fn undo(&mut self) -> Option<&str> {
        if self.cursor == 0 {
            return None;
        }
        self.cursor -= 1;
        Some(self.current())
    }

    pub(crate) fn redo(&mut self) -> Option<&str> {
        if self.cursor + 1 >= self.snapshots.len() {
            return None;
        }
        self.cursor += 1;
        Some(self.current())
    }

    pub(crate) fn is_dirty(&self) -> bool {
        self.saved != Some(self.cursor)
    }

    pub(crate) fn mark_saved(&mut self) {
        self.saved = Some(self.cursor);
    }
}
