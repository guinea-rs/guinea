/// Which row has the focus.
///
/// A field of the page that draws the list, not a reducer: nothing outside
/// the page reads it, so it lives as long as the page and dies with it.
#[derive(Default, Clone, Copy, PartialEq, Debug)]
pub struct Cursor {
    row: usize,
}

impl Cursor {
    /// The focused row in a list of `len` - the list is refreshed by an actor
    /// and can shrink under the focus.
    pub fn row(&mut self, len: usize) -> usize {
        self.step(0, len);
        self.row
    }

    pub fn step(&mut self, delta: isize, len: usize) {
        let Some(last) = len.checked_sub(1) else {
            self.row = 0;
            return;
        };
        let next = (self.row as isize).saturating_add(delta);
        self.row = next.clamp(0, last as isize) as usize;
    }
}
