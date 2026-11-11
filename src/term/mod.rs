pub mod render;
pub mod theme;

use alacritty_terminal::event::VoidListener;
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::term::{Config, RenderableContent, Term};
use alacritty_terminal::vte::ansi::Processor;

#[derive(Clone, Copy)]
struct Size {
    cols: usize,
    rows: usize,
    scrollback: usize,
}

impl Dimensions for Size {
    fn total_lines(&self) -> usize {
        self.rows + self.scrollback
    }
    fn screen_lines(&self) -> usize {
        self.rows
    }
    fn columns(&self) -> usize {
        self.cols
    }
}

/// Owns the alacritty VTE/grid engine for one terminal tab (local or SSH —
/// both just feed raw bytes in here the same way).
pub struct TerminalSurface {
    term: Term<VoidListener>,
    parser: Processor,
    size: Size,
}

impl TerminalSurface {
    pub fn new(cols: usize, rows: usize) -> Self {
        let size = Size {
            cols: cols.max(1),
            rows: rows.max(1),
            scrollback: 10_000,
        };
        let term = Term::new(Config::default(), &size, VoidListener);
        Self {
            term,
            parser: Processor::new(),
            size,
        }
    }

    pub fn feed(&mut self, bytes: &[u8]) {
        self.parser.advance(&mut self.term, bytes);
    }

    pub fn cols(&self) -> usize {
        self.size.cols
    }

    pub fn rows(&self) -> usize {
        self.size.rows
    }

    pub fn resize(&mut self, cols: usize, rows: usize) {
        if cols == 0 || rows == 0 || (cols == self.size.cols && rows == self.size.rows) {
            return;
        }
        self.size.cols = cols;
        self.size.rows = rows;
        self.term.resize(self.size);
    }

    pub fn renderable(&self) -> RenderableContent<'_> {
        self.term.renderable_content()
    }
}
