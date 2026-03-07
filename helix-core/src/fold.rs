//! Fold functionality for collapsing and expanding code regions.
//!
//! This module provides utilities for detecting foldable regions using
//! Tree-sitter queries and managing fold state.

use helix_loader::grammar::get_language;
use ropey::RopeSlice;
use std::cmp::Reverse;
use tree_house::tree_sitter::{query::Query, RopeInput};

use crate::syntax::{self, Syntax};

/// Represents a foldable region in the document.
#[derive(Debug, Clone)]
pub struct FoldRegion {
    /// The starting line of the fold (0-indexed).
    pub start_line: usize,
    /// The ending line of the fold (0-indexed).
    pub end_line: usize,
    /// The kind of fold.
    pub kind: FoldKind,
    /// The nesting depth of this fold.
    pub depth: usize,
}

/// The type of fold region.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FoldKind {
    /// A comment block.
    Comment,
    /// Import statements.
    Imports,
    /// A user-defined region (e.g., #pragma region in C#).
    Region,
    /// A syntax-based fold (e.g., functions, if statements).
    Syntax,
}

impl FoldKind {
    /// Infer fold kind from the node's type name.
    pub fn from_node_type(node_type: &str) -> Self {
        let lower = node_type.to_lowercase();
        if lower.contains("comment") {
            FoldKind::Comment
        } else if lower.contains("import") || lower.contains("include") {
            FoldKind::Imports
        } else if lower.contains("region") {
            FoldKind::Region
        } else {
            FoldKind::Syntax
        }
    }
}

/// Get all foldable regions in the document using Tree-sitter queries.
pub fn get_foldable_ranges(
    syntax: &Syntax,
    source: RopeSlice,
    language_name: &str,
    grammar_name: &str,
) -> Vec<FoldRegion> {
    let query_text = syntax::read_query(language_name, "folds.scm");

    if query_text.is_empty() {
        return Vec::new();
    }

    // Get grammar from language name
    let Some(grammar) = get_language(grammar_name).ok().flatten() else {
        return Vec::new();
    };

    let query = match Query::new(grammar, &query_text, |_, _| Ok(())) {
        Ok(query) => query,
        Err(e) => {
            log::warn!("Failed to parse folds.scm query: {:?}", e);
            return Vec::new();
        }
    };

    // Check if there's a @fold capture in the query
    let Some(fold_capture) = query.get_capture("fold") else {
        return Vec::new();
    };

    let mut regions = Vec::new();
    let mut cursor = tree_house::tree_sitter::InactiveQueryCursor::new(
        0..u32::MAX,
        crate::syntax::TREE_SITTER_MATCH_LIMIT,
    )
    .execute_query(&query, &syntax.tree().root_node(), RopeInput::new(source));

    while let Some(m) = cursor.next_match() {
        // Look for @fold captures in this match
        for node in m.nodes_for_capture(fold_capture) {
            let byte_range = node.byte_range();

            // Convert byte positions to line numbers
            let start_line = source.byte_to_line(byte_range.start as usize);
            let end_line = source.byte_to_line((byte_range.end as usize).saturating_sub(1));

            if end_line > start_line {
                let kind = FoldKind::from_node_type(node.kind());
                regions.push(FoldRegion {
                    start_line,
                    end_line,
                    kind,
                    depth: 0,
                });
            }
        }
    }

    // Sort by start line, then by end line descending (larger regions first)
    regions.sort_by(|a, b| {
        a.start_line
            .cmp(&b.start_line)
            .then(b.end_line.cmp(&a.end_line))
    });

    // Remove exact duplicates (same start and end line) but keep nested regions
    regions.dedup_by(|a, b| a.start_line == b.start_line && a.end_line == b.end_line);

    assign_fold_depths(&mut regions);

    regions
}

/// Assign nesting depth based on fold regions rather than raw AST parent depth.
///
/// Regions are expected to be sorted by `start_line ASC, end_line DESC`.
fn assign_fold_depths(regions: &mut [FoldRegion]) {
    let mut active_ancestors: Vec<(usize, usize)> = Vec::new();

    for region in regions.iter_mut() {
        while let Some((ancestor_start, ancestor_end)) = active_ancestors.last() {
            let starts_after_ancestor = region.start_line > *ancestor_end;
            let not_contained_by_ancestor = region.end_line > *ancestor_end;
            let same_span =
                region.start_line == *ancestor_start && region.end_line == *ancestor_end;
            if starts_after_ancestor || not_contained_by_ancestor || same_span {
                active_ancestors.pop();
            } else {
                break;
            }
        }

        region.depth = active_ancestors.len() + 1;
        active_ancestors.push((region.start_line, region.end_line));
    }
}

/// Fold state for a single view.
#[derive(Debug, Clone, Default)]
pub struct FoldState {
    /// Currently folded regions (by start line).
    pub folded: Vec<usize>,
    /// Current fold level if folds are managed via level-based commands.
    pub fold_level: Option<usize>,
}

impl FoldState {
    /// Create a new fold state.
    pub fn new() -> Self {
        Self {
            folded: Vec::new(),
            fold_level: None,
        }
    }

    /// Check if a line is folded.
    pub fn is_folded(&self, line: usize) -> bool {
        self.folded.contains(&line)
    }

    /// Toggle fold state for a line.
    pub fn toggle(&mut self, line: usize) -> bool {
        self.fold_level = None;
        if let Some(pos) = self.folded.iter().position(|&l| l == line) {
            self.folded.remove(pos);
            false
        } else {
            self.folded.push(line);
            true
        }
    }

    /// Fold a line.
    pub fn fold(&mut self, line: usize) {
        self.fold_level = None;
        if !self.folded.contains(&line) {
            self.folded.push(line);
            self.folded.sort();
        }
    }

    /// Unfold a line.
    pub fn unfold(&mut self, line: usize) {
        self.fold_level = None;
        if let Some(pos) = self.folded.iter().position(|&l| l == line) {
            self.folded.remove(pos);
        }
    }

    /// Unfold all lines.
    pub fn unfold_all(&mut self) {
        self.fold_level = None;
        self.folded.clear();
    }

    /// Fold all provided regions.
    pub fn fold_all(&mut self, regions: &[FoldRegion]) {
        self.fold_level = None;
        self.folded = regions.iter().map(|r| r.start_line).collect();
    }

    /// Fold to a specific level (Vim-like foldlevel semantics).
    /// Regions deeper than `level` are folded; shallower/equal are unfolded.
    pub fn fold_to_level(&mut self, regions: &[FoldRegion], level: usize) {
        self.fold_level = Some(level);
        self.folded = regions
            .iter()
            .filter(|region| region.depth > level)
            .map(|region| region.start_line)
            .collect();
        self.folded.sort_unstable();
        self.folded.dedup();
    }

    pub fn level(&self) -> Option<usize> {
        self.fold_level
    }
}

/// Get the fold region that contains a specific line.
pub fn get_fold_at_line(regions: &[FoldRegion], line: usize) -> Option<FoldRegion> {
    regions
        .iter()
        .filter(|r| line >= r.start_line && line <= r.end_line)
        // Prefer the innermost containing fold.
        .max_by_key(|r| (r.start_line, Reverse(r.end_line)))
        .cloned()
}

/// Get all folds that start at or after a specific line.
pub fn get_folds_after(regions: &[FoldRegion], line: usize) -> Vec<FoldRegion> {
    regions
        .iter()
        .filter(|r| r.start_line >= line)
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fold_kind_from_node_type() {
        assert_eq!(FoldKind::from_node_type("comment"), FoldKind::Comment);
        assert_eq!(FoldKind::from_node_type("block_comment"), FoldKind::Comment);
        assert_eq!(
            FoldKind::from_node_type("import_statement"),
            FoldKind::Imports
        );
        assert_eq!(FoldKind::from_node_type("include"), FoldKind::Imports);
        assert_eq!(
            FoldKind::from_node_type("function_declaration"),
            FoldKind::Syntax
        );
        assert_eq!(FoldKind::from_node_type("if_statement"), FoldKind::Syntax);
    }

    #[test]
    fn test_fold_state() {
        let mut state = FoldState::new();

        assert!(!state.is_folded(10));
        assert_eq!(state.level(), None);

        state.fold(10);
        assert!(state.is_folded(10));

        state.unfold(10);
        assert!(!state.is_folded(10));

        state.toggle(10);
        assert!(state.is_folded(10));

        state.toggle(10);
        assert!(!state.is_folded(10));
        assert_eq!(state.level(), None);

        let regions = vec![
            FoldRegion {
                start_line: 1,
                end_line: 5,
                kind: FoldKind::Syntax,
                depth: 1,
            },
            FoldRegion {
                start_line: 2,
                end_line: 4,
                kind: FoldKind::Syntax,
                depth: 2,
            },
        ];
        state.fold_to_level(&regions, 1);
        assert_eq!(state.level(), Some(1));
        assert!(!state.is_folded(1));
        assert!(state.is_folded(2));
    }

    #[test]
    fn test_assign_fold_depths_uses_fold_hierarchy() {
        let mut regions = vec![
            FoldRegion {
                start_line: 0,
                end_line: 20,
                kind: FoldKind::Syntax,
                depth: 0,
            },
            FoldRegion {
                start_line: 3,
                end_line: 10,
                kind: FoldKind::Syntax,
                depth: 0,
            },
            FoldRegion {
                start_line: 5,
                end_line: 8,
                kind: FoldKind::Syntax,
                depth: 0,
            },
            FoldRegion {
                start_line: 12,
                end_line: 18,
                kind: FoldKind::Syntax,
                depth: 0,
            },
        ];

        assign_fold_depths(&mut regions);
        let depths: Vec<usize> = regions.iter().map(|r| r.depth).collect();
        assert_eq!(depths, vec![1, 2, 3, 2]);
    }

    #[test]
    fn test_get_fold_at_line_prefers_innermost_region() {
        let regions = vec![
            FoldRegion {
                start_line: 0,
                end_line: 20,
                kind: FoldKind::Syntax,
                depth: 1,
            },
            FoldRegion {
                start_line: 3,
                end_line: 10,
                kind: FoldKind::Syntax,
                depth: 2,
            },
            FoldRegion {
                start_line: 5,
                end_line: 8,
                kind: FoldKind::Syntax,
                depth: 3,
            },
        ];

        let selected = get_fold_at_line(&regions, 6).expect("expected fold at line");
        assert_eq!(selected.start_line, 5);
        assert_eq!(selected.end_line, 8);
    }
}
