//! Fold annotations for rendering folded code regions.

use helix_core::doc_formatter::FormattedGrapheme;
use helix_core::fold::{FoldKind, FoldRegion, FoldState};
use helix_core::text_annotations::LineAnnotation;
use helix_core::Position;

use crate::Document;

/// Configuration for fold annotations.
#[derive(Debug, Clone, Default)]
pub struct FoldConfig {
    /// Whether to show fold indicators.
    pub enabled: bool,
    /// The character to show for folded regions.
    pub indicator: char,
}

impl FoldConfig {
    pub fn new() -> Self {
        Self {
            enabled: true,
            indicator: '▶',
        }
    }
}

/// Fold annotations state.
pub struct FoldAnnotations<'a> {
    /// The document being annotated.
    doc: &'a Document,
    /// The view ID for fold state.
    view_id: crate::ViewId,
    /// Cached foldable regions.
    regions: Vec<FoldRegion>,
    /// Current position in the document.
    char_idx: usize,
    /// Next anchor position to watch.
    next_anchor: usize,
    /// Fold configuration (e.g. for future indicator character).
    #[allow(dead_code)]
    config: FoldConfig,
}

impl<'a> FoldAnnotations<'a> {
    pub fn new(doc: &'a Document, view_id: crate::ViewId, config: FoldConfig) -> Self {
        // Initialize fold regions from syntax
        let regions = if let Some(syntax) = &doc.syntax {
            let text = doc.text().slice(..);
            let language_name = doc.language_name().unwrap_or_default();
            let grammar_name = doc
                .language_config()
                .and_then(|config| config.grammar.as_deref())
                .unwrap_or(language_name);
            helix_core::fold::get_foldable_ranges(syntax, text, language_name, grammar_name)
        } else {
            Vec::new()
        };

        Self {
            doc,
            view_id,
            regions,
            char_idx: 0,
            next_anchor: usize::MAX,
            config,
        }
    }

    /// Get the fold state for the current view.
    fn fold_state(&self) -> Option<&FoldState> {
        self.doc.folds.get(&self.view_id)
    }

    /// Get a folded region that contains `line`.
    ///
    /// Prefer the outermost containing region so concealed content remains
    /// hidden even when rendering starts from inside a folded block.
    fn folded_region_containing_line(&self, line: usize) -> Option<&FoldRegion> {
        let fold_state = self.fold_state()?;
        self.regions
            .iter()
            .filter(|region| {
                line >= region.start_line
                    && line <= region.end_line
                    && fold_state.is_folded(region.start_line)
            })
            .min_by_key(|region| region.start_line)
    }

    fn should_keep_end_line_visible(&self, region: &FoldRegion) -> bool {
        if region.kind != FoldKind::Syntax {
            return false;
        }

        let text = self.doc.text().slice(..);
        if region.end_line >= text.len_lines() {
            return false;
        }
        let end_line = text
            .line(region.end_line)
            .chars()
            .filter(|c| *c != '\n' && *c != '\r')
            .collect::<String>();
        let trimmed = end_line.trim();
        matches!(
            trimmed,
            "}" | "};" | "}," | "]" | "];" | "]," | ")" | ");" | "),"
        )
    }

    /// Get the next interesting position (where a fold starts).
    #[allow(dead_code)]
    fn next_fold_position(&self, from_line: usize) -> Option<usize> {
        self.fold_state().and_then(|state| {
            state
                .folded
                .iter()
                .find(|&&line| line >= from_line)
                .copied()
        })
    }
}

impl LineAnnotation for FoldAnnotations<'_> {
    fn reset_pos(&mut self, char_idx: usize) -> usize {
        self.char_idx = char_idx;
        self.next_anchor = usize::MAX;
        self.next_anchor
    }

    fn process_anchor(&mut self, _grapheme: &FormattedGrapheme) -> usize {
        // For now, we don't need to process anchors for folds
        self.next_anchor
    }

    fn insert_virtual_lines(
        &mut self,
        _line_end_char_idx: usize,
        _line_end_visual_pos: Position,
        _doc_line: usize,
    ) -> Position {
        // No extra virtual lines needed for folds; the fold overlay text is
        // rendered at the end of the fold line by FoldDecoration.
        Position::new(0, 0)
    }

    fn lines_to_skip_after_line(&self, doc_line: usize) -> Option<usize> {
        let Some(region) = self.folded_region_containing_line(doc_line) else {
            return None;
        };

        let skip_until = if self.should_keep_end_line_visible(region) {
            region.end_line.saturating_sub(1)
        } else {
            region.end_line
        };
        let lines_to_skip = skip_until.saturating_sub(doc_line);
        (lines_to_skip > 0).then_some(lines_to_skip)
    }
}
