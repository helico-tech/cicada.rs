//! The one open window of an embedded inspector.

use std::cell::RefCell;

pub(crate) struct WindowSlot<W> {
    open: RefCell<Option<W>>,
}

impl<W> Default for WindowSlot<W> {
    fn default() -> Self {
        Self { open: RefCell::default() }
    }
}

impl<W: Clone> WindowSlot<W> {
    /// Returns the open window, creating it with a fresh session only on the first show after close.
    pub(crate) fn show(&self, make: impl FnOnce() -> W) -> W {
        let open = self.open.borrow().clone();
        if let Some(window) = open {
            return window;
        }
        let window = make();
        self.open.replace(Some(window.clone()));
        window
    }

    /// Releases ownership for a host-initiated close.
    pub(crate) fn take(&self) -> Option<W> {
        self.open.take()
    }

    /// Releases ownership on a native close of this window. A close of an earlier window leaves a new one alone.
    pub(crate) fn closed(&self, is_window: impl FnOnce(&W) -> bool) {
        let matches = self.open.borrow().as_ref().is_some_and(is_window);
        if matches {
            self.take();
        }
    }
}
