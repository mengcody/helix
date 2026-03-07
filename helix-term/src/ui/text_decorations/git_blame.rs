use std::{path::PathBuf, sync::Arc};

use arc_swap::ArcSwap;
use helix_core::Position;
use helix_vcs::{DiffProviderRegistry, LineBlameStatus};
use helix_view::theme::Style;
use helix_view::{Document, Editor, Theme, View};

use crate::ui::document::{LinePos, TextRenderer};
use crate::ui::text_decorations::Decoration;

pub struct InlineGitBlame {
    diff_providers: DiffProviderRegistry,
    line_blame: Arc<ArcSwap<LineBlameStatus>>,
    path: Option<PathBuf>,
    cursor_line: usize,
    style: Style,
}

impl InlineGitBlame {
    pub fn new(editor: &Editor, doc: &Document, view: &View, theme: &Theme) -> Self {
        let text = doc.text().slice(..);
        let cursor_line = doc.selection(view.id).primary().cursor_line(text);
        let style = theme
            .try_get("ui.virtual")
            .or_else(|| theme.try_get("ui.virtual.inlay-hint"))
            .unwrap_or_else(|| theme.get("ui.text.info"));

        Self {
            diff_providers: editor.diff_providers.clone(),
            line_blame: doc.line_blame(),
            path: doc.path().map(|path| path.to_path_buf()),
            cursor_line,
            style,
        }
    }
}

impl Decoration for InlineGitBlame {
    fn render_virt_lines(
        &mut self,
        renderer: &mut TextRenderer,
        pos: LinePos,
        virt_off: Position,
    ) -> Position {
        if !pos.first_visual_line || pos.doc_line != self.cursor_line {
            return Position::new(0, 0);
        }

        let Some(path) = self.path.clone() else {
            return Position::new(0, 0);
        };

        let state = self.line_blame.load_full();
        if state.line != Some(self.cursor_line) {
            spawn_line_blame_update(
                self.diff_providers.clone(),
                self.line_blame.clone(),
                path,
                self.cursor_line,
            );
            return Position::new(0, 0);
        }

        if state.loading {
            return Position::new(0, 0);
        }

        let Some(message) = state.message.as_deref() else {
            return Position::new(0, 0);
        };

        let draw_col = (virt_off.col + 1) as u16;
        if !renderer.column_in_bounds(draw_col as usize, 1) {
            return Position::new(0, 0);
        }

        let (end_col, _) = renderer.set_string_truncated(
            renderer.viewport.x + draw_col,
            pos.visual_line,
            &format!(" {}", message),
            renderer.viewport.width.saturating_sub(draw_col) as usize,
            |_| self.style,
            true,
            false,
        );

        let used = end_col.saturating_sub(renderer.viewport.x + draw_col);
        Position::new(0, used as usize)
    }
}

fn spawn_line_blame_update(
    diff_providers: DiffProviderRegistry,
    line_blame: Arc<ArcSwap<LineBlameStatus>>,
    path: PathBuf,
    line: usize,
) {
    line_blame.store(Arc::new(LineBlameStatus {
        line: Some(line),
        loading: true,
        message: None,
    }));

    tokio::task::spawn_blocking(move || {
        let message = diff_providers.get_line_blame(&path, line);
        let current = line_blame.load_full();
        if current.line != Some(line) {
            return;
        }

        line_blame.store(Arc::new(LineBlameStatus {
            line: Some(line),
            loading: false,
            message,
        }));
        helix_event::request_redraw();
    });
}
