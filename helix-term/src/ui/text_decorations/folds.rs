use helix_core::fold::{self, FoldRegion};
use helix_core::Position;
use helix_view::theme::Style;
use helix_view::{Document, Theme, ViewId};

use crate::ui::document::{LinePos, TextRenderer};
use crate::ui::text_decorations::Decoration;

/// Decoration that renders fold overlay text at the end of folded lines.
pub struct FoldDecoration<'a> {
    doc: &'a Document,
    view_id: ViewId,
    style: Style,
    regions: Vec<FoldRegion>,
}

impl<'a> FoldDecoration<'a> {
    pub fn new(doc: &'a Document, view_id: ViewId, theme: &Theme) -> Self {
        let regions = if let Some(syntax) = &doc.syntax {
            let text = doc.text().slice(..);
            let language_name = doc.language_name().unwrap_or_default();
            let grammar_name = doc
                .language_config()
                .and_then(|config| config.grammar.as_deref())
                .unwrap_or(language_name);
            fold::get_foldable_ranges(syntax, text, language_name, grammar_name)
        } else {
            Vec::new()
        };

        let style = theme.try_get("ui.virtual.fold").unwrap_or_else(|| {
            theme
                .try_get("ui.virtual")
                .unwrap_or_else(|| theme.get("ui.text.info"))
        });

        Self {
            doc,
            view_id,
            style,
            regions,
        }
    }

    /// Check if a line is folded.
    fn is_folded(&self, line: usize) -> bool {
        self.doc
            .fold_state(self.view_id)
            .map(|state| state.is_folded(line))
            .unwrap_or(false)
    }

    /// Get the fold region starting at a specific line.
    fn get_fold_region(&self, line: usize) -> Option<&FoldRegion> {
        self.regions.iter().find(|r| r.start_line == line)
    }
}

impl Decoration for FoldDecoration<'_> {
    fn render_virt_lines(
        &mut self,
        renderer: &mut TextRenderer,
        pos: LinePos,
        virt_off: Position,
    ) -> Position {
        if !pos.first_visual_line || !self.is_folded(pos.doc_line) {
            return Position::new(0, 0);
        }

        let Some(region) = self.get_fold_region(pos.doc_line) else {
            return Position::new(0, 0);
        };

        let hidden_lines = region.end_line - region.start_line;
        let overlay = format!(" ▶ {} lines ", hidden_lines);

        let draw_col = (virt_off.col + 1) as u16;
        let width = renderer.viewport.width;

        if !renderer.column_in_bounds(draw_col as usize, 1) {
            return Position::new(0, 0);
        }

        let style = self.style;
        let (end_col, _) = renderer.set_string_truncated(
            renderer.viewport.x + draw_col,
            pos.visual_line,
            &overlay,
            width.saturating_sub(draw_col) as usize,
            |_| style,
            true,
            false,
        );

        let used = end_col.saturating_sub(renderer.viewport.x + draw_col);
        Position::new(0, used as usize)
    }
}
