//! The emitter lowering writes one tree body through: `.air` lines at a block depth, plus marks
//! for trial lowering (rewind to a mark, split off or read what was written since one, take the
//! text out and put it back).

/// A tree body under construction. Lines are indented 4 spaces per depth level; a body starts at
/// depth 1 (inside the tree's braces).
#[derive(Debug, Clone)]
pub(crate) struct Emitter {
    text: String,
    depth: usize,
}

impl Emitter {
    pub(crate) fn new(depth: usize) -> Self {
        Self {
            text: String::new(),
            depth,
        }
    }

    /// Writes one line at the current depth.
    pub(crate) fn line(&mut self, text: &str) {
        for _ in 0..self.depth {
            self.text.push_str("    ");
        }
        self.text.push_str(text);
        self.text.push('\n');
    }

    /// Enters a nested block.
    pub(crate) fn indent(&mut self) {
        self.depth += 1;
    }

    /// Leaves a nested block.
    pub(crate) fn dedent(&mut self) {
        self.depth -= 1;
    }

    pub(crate) fn depth(&self) -> usize {
        self.depth
    }

    /// Sets the depth, returning the previous one.
    pub(crate) fn set_depth(&mut self, depth: usize) -> usize {
        std::mem::replace(&mut self.depth, depth)
    }

    /// A position to come back to: everything written after it can be dropped or split off.
    pub(crate) fn mark(&self) -> usize {
        self.text.len()
    }

    /// Drops everything written after `mark`.
    pub(crate) fn truncate(&mut self, mark: usize) {
        self.text.truncate(mark);
    }

    /// Removes and returns everything written after `mark`.
    pub(crate) fn split_off(&mut self, mark: usize) -> String {
        self.text.split_off(mark)
    }

    /// Everything written after `mark`.
    pub(crate) fn since(&self, mark: usize) -> &str {
        &self.text[mark..]
    }

    pub(crate) fn text(&self) -> &str {
        &self.text
    }

    /// Takes the text out, leaving the emitter empty (at the same depth).
    pub(crate) fn take_text(&mut self) -> String {
        std::mem::take(&mut self.text)
    }

    /// Replaces the text (after `take_text`).
    pub(crate) fn set_text(&mut self, text: String) {
        self.text = text;
    }

    /// Appends already-indented lines.
    pub(crate) fn append(&mut self, text: &str) {
        self.text.push_str(text);
    }

    pub(crate) fn into_text(self) -> String {
        self.text
    }
}
