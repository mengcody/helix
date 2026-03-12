use crate::{
    compositor::{Callback, Component, Context, Event, EventResult},
    ctrl, key,
};
use helix_view::{
    graphics::{Margin, Modifier, Rect},
    theme::Style,
};
use tui::{
    buffer::Buffer as Surface,
    widgets::{Block, Widget},
};

#[derive(Clone)]
pub struct DiffViewerData {
    pub title: String,
    pub source_path: String,
    pub rows: Vec<DiffRow>,
    pub hunk_row_indices: Vec<usize>,
    pub preferred_row: usize,
}

#[derive(Clone)]
pub enum DiffRow {
    FileHeader {
        path: String,
        added: usize,
        removed: usize,
    },
    HunkHeader {
        text: String,
    },
    Context {
        left_no: Option<u32>,
        right_no: Option<u32>,
        left: String,
        right: String,
    },
    Delete {
        left_no: u32,
        left: String,
    },
    Insert {
        right_no: u32,
        right: String,
    },
    Modify {
        left_no: u32,
        right_no: u32,
        left: String,
        right: String,
    },
    Spacer,
}

pub struct DiffViewer {
    data: DiffViewerData,
    scroll: usize,
}

impl DiffViewer {
    pub const ID: &'static str = "diff-viewer";

    pub fn new(data: DiffViewerData) -> Self {
        Self {
            scroll: data.preferred_row,
            data,
        }
    }

    pub fn source_path(&self) -> &str {
        &self.data.source_path
    }

    pub fn set_data(&mut self, data: DiffViewerData) {
        self.scroll = data.preferred_row;
        self.data = data;
    }

    fn current_hunk_index(&self) -> Option<usize> {
        if self.data.hunk_row_indices.is_empty() {
            return None;
        }

        let idx = self
            .data
            .hunk_row_indices
            .partition_point(|row| *row <= self.scroll);
        Some(idx.saturating_sub(1))
    }

    pub fn jump_next_hunk(&mut self) -> Option<(usize, usize)> {
        let next_idx = self
            .data
            .hunk_row_indices
            .iter()
            .position(|row| *row > self.scroll)?;
        self.scroll = self.data.hunk_row_indices[next_idx];
        Some((next_idx + 1, self.data.hunk_row_indices.len()))
    }

    pub fn jump_prev_hunk(&mut self) -> Option<(usize, usize)> {
        let prev_idx = self
            .data
            .hunk_row_indices
            .iter()
            .rposition(|row| *row < self.scroll)?;
        self.scroll = self.data.hunk_row_indices[prev_idx];
        Some((prev_idx + 1, self.data.hunk_row_indices.len()))
    }

    pub fn current_hunk_status(&self) -> Option<(usize, usize)> {
        let idx = self.current_hunk_index()?;
        Some((idx + 1, self.data.hunk_row_indices.len()))
    }

    fn current_hunk_bounds(&self) -> Option<(usize, usize)> {
        let idx = self.current_hunk_index()?;
        let start = *self.data.hunk_row_indices.get(idx)?;
        let end = self
            .data
            .hunk_row_indices
            .get(idx + 1)
            .copied()
            .unwrap_or(self.data.rows.len());
        Some((start, end))
    }

    fn close_callback() -> Callback {
        Box::new(|compositor, _| {
            compositor.remove(Self::ID);
        })
    }

    fn max_scroll(&self, viewport_rows: usize) -> usize {
        self.data.rows.len().saturating_sub(viewport_rows)
    }

    fn scroll_by(&mut self, delta: isize, viewport_rows: usize) {
        let max_scroll = self.max_scroll(viewport_rows);
        self.scroll = self.scroll.saturating_add_signed(delta).min(max_scroll);
    }

    fn render_title(&self, area: Rect, surface: &mut Surface, cx: &Context) {
        let title_style = cx.editor.theme.get("ui.text");
        let subtle_style = cx.editor.theme.get("ui.text.info");

        let hunk_text = match self.current_hunk_status() {
            Some((idx, total)) => format!("Hunk {idx}/{total}"),
            None => "No hunks".to_string(),
        };
        let summary = format!("Diff Viewer  {}  {}", self.data.source_path, hunk_text);
        surface.set_stringn(area.x, area.y, summary, area.width as usize, title_style);

        if area.height > 1 {
            let controls = "j/k scroll  [h ]h hunks  [H ]H files  q close";
            surface.set_stringn(
                area.x,
                area.y + 1,
                controls,
                area.width as usize,
                subtle_style,
            );
        }
    }

    fn render_column_headers(&self, area: Rect, surface: &mut Surface, cx: &Context) {
        if area.width < 3 {
            return;
        }

        let header_style = cx.editor.theme.get("ui.text.info");
        let separator_style = cx.editor.theme.get("ui.window");
        let separator_x = area.x + area.width / 2;
        let left_width = separator_x.saturating_sub(area.x);
        let right_x = separator_x.saturating_add(1);
        let right_width = area.right().saturating_sub(right_x);

        surface.set_style(area, header_style);
        surface.set_style(Rect::new(separator_x, area.y, 1, 1), separator_style);
        surface.set_string(separator_x, area.y, "│", separator_style);
        if left_width > 0 {
            surface.set_stringn(area.x, area.y, " OLD", left_width as usize, header_style);
        }
        if right_width > 0 {
            surface.set_stringn(right_x, area.y, " NEW", right_width as usize, header_style);
        }
    }

    fn render_split_row(
        &self,
        area: Rect,
        surface: &mut Surface,
        left_no: Option<u32>,
        right_no: Option<u32>,
        left: &str,
        right: &str,
        left_style: Style,
        right_style: Style,
        gutter_style: Style,
        separator_style: Style,
        inline_highlight: Option<(&str, &str)>,
    ) {
        if area.width == 0 {
            return;
        }

        let separator_x = area.x + area.width / 2;
        let left_width = separator_x.saturating_sub(area.x);
        let right_x = separator_x.saturating_add(1);
        let right_width = area.right().saturating_sub(right_x);

        if left_width > 0 {
            surface.set_style(Rect::new(area.x, area.y, left_width, 1), left_style);
        }
        if right_width > 0 {
            surface.set_style(Rect::new(right_x, area.y, right_width, 1), right_style);
        }
        surface.set_style(Rect::new(separator_x, area.y, 1, 1), separator_style);
        surface.set_string(separator_x, area.y, "│", separator_style);

        let line_no_width = 5usize;
        let left_label = left_no.map_or_else(|| " ".repeat(line_no_width), |n| format!("{n:>5}"));
        let right_label = right_no.map_or_else(|| " ".repeat(line_no_width), |n| format!("{n:>5}"));

        surface.set_stringn(
            area.x,
            area.y,
            &left_label,
            left_width as usize,
            gutter_style,
        );
        if left_width as usize > line_no_width + 1 {
            self.render_content(
                surface,
                area.x + line_no_width as u16 + 1,
                area.y,
                left_width as usize - line_no_width - 1,
                left,
                left_style,
                inline_highlight.map(|(left, right)| (left, right, true)),
            );
        }

        if right_width > 0 {
            surface.set_stringn(
                right_x,
                area.y,
                &right_label,
                right_width as usize,
                gutter_style,
            );
            if right_width as usize > line_no_width + 1 {
                self.render_content(
                    surface,
                    right_x + line_no_width as u16 + 1,
                    area.y,
                    right_width as usize - line_no_width - 1,
                    right,
                    right_style,
                    inline_highlight.map(|(left, right)| (left, right, false)),
                );
            }
        }
    }

    fn render_content(
        &self,
        surface: &mut Surface,
        x: u16,
        y: u16,
        width: usize,
        text: &str,
        base_style: Style,
        inline_highlight: Option<(&str, &str, bool)>,
    ) {
        let Some((left, right, is_left)) = inline_highlight else {
            surface.set_stringn(x, y, truncate_with_ellipsis(text, width), width, base_style);
            return;
        };

        let segments = inline_diff_segments(left, right);
        let (prefix, changed, suffix) = if is_left {
            (segments.prefix, segments.left_changed, segments.suffix)
        } else {
            (segments.prefix, segments.right_changed, segments.suffix)
        };

        let highlight_style = base_style.add_modifier(Modifier::BOLD);
        let mut col = x;
        let mut remaining = width;

        for (segment, style) in [
            (prefix.as_str(), base_style),
            (changed.as_str(), highlight_style),
            (suffix.as_str(), base_style),
        ] {
            if remaining == 0 || segment.is_empty() {
                continue;
            }
            let rendered_width = surface
                .set_stringn(
                    col,
                    y,
                    truncate_with_ellipsis(segment, remaining),
                    remaining,
                    style,
                )
                .0;
            let consumed = rendered_width.saturating_sub(col) as usize;
            col = rendered_width;
            remaining = remaining.saturating_sub(consumed);
        }
    }
}

struct InlineDiffSegments {
    prefix: String,
    left_changed: String,
    right_changed: String,
    suffix: String,
}

fn inline_diff_segments(left: &str, right: &str) -> InlineDiffSegments {
    let left_chars: Vec<char> = left.chars().collect();
    let right_chars: Vec<char> = right.chars().collect();

    let mut prefix_len = 0;
    while prefix_len < left_chars.len()
        && prefix_len < right_chars.len()
        && left_chars[prefix_len] == right_chars[prefix_len]
    {
        prefix_len += 1;
    }

    let mut suffix_len = 0;
    while suffix_len < left_chars.len().saturating_sub(prefix_len)
        && suffix_len < right_chars.len().saturating_sub(prefix_len)
        && left_chars[left_chars.len() - 1 - suffix_len]
            == right_chars[right_chars.len() - 1 - suffix_len]
    {
        suffix_len += 1;
    }

    let left_mid_end = left_chars.len().saturating_sub(suffix_len);
    let right_mid_end = right_chars.len().saturating_sub(suffix_len);

    InlineDiffSegments {
        prefix: left_chars[..prefix_len].iter().collect(),
        left_changed: left_chars[prefix_len..left_mid_end].iter().collect(),
        right_changed: right_chars[prefix_len..right_mid_end].iter().collect(),
        suffix: left_chars[left_mid_end..].iter().collect(),
    }
}

fn truncate_with_ellipsis(text: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }

    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= width {
        return text.to_string();
    }
    if width == 1 {
        return "…".to_string();
    }

    let mut truncated: String = chars.into_iter().take(width - 1).collect();
    truncated.push('…');
    truncated
}

impl Component for DiffViewer {
    fn handle_event(&mut self, event: &Event, _cx: &mut Context) -> EventResult {
        let key = match event {
            Event::Key(key) => *key,
            Event::Resize(_, _) => return EventResult::Consumed(None),
            _ => return EventResult::Ignored(None),
        };

        let viewport_rows = 10usize;
        match key {
            key!('q') | key!(Esc) => EventResult::Consumed(Some(Self::close_callback())),
            key!('j') | key!(Down) => {
                self.scroll_by(1, viewport_rows);
                EventResult::Consumed(None)
            }
            key!('k') | key!(Up) => {
                self.scroll_by(-1, viewport_rows);
                EventResult::Consumed(None)
            }
            key!(PageDown) | ctrl!('d') => {
                self.scroll_by(viewport_rows as isize / 2, viewport_rows);
                EventResult::Consumed(None)
            }
            key!(PageUp) | ctrl!('u') => {
                self.scroll_by(-(viewport_rows as isize / 2), viewport_rows);
                EventResult::Consumed(None)
            }
            key!(Home) => {
                self.scroll = 0;
                EventResult::Consumed(None)
            }
            key!(End) => {
                self.scroll = self.max_scroll(viewport_rows);
                EventResult::Consumed(None)
            }
            _ => EventResult::Ignored(None),
        }
    }

    fn render(&mut self, viewport: Rect, surface: &mut Surface, cx: &mut Context) {
        let area = viewport.clip_bottom(2);
        let block_style = cx.editor.theme.get("ui.popup");
        let subtle_style = cx.editor.theme.get("ui.text.info");
        let context_style = cx.editor.theme.get("ui.text");
        let gutter_style = cx.editor.theme.get("ui.linenr");
        let separator_style = cx.editor.theme.get("ui.window");
        let plus_style = cx.editor.theme.get("diff.plus");
        let minus_style = cx.editor.theme.get("diff.minus");
        let header_style = subtle_style;
        let current_header_style = subtle_style.add_modifier(Modifier::BOLD);
        let current_context_style = context_style.add_modifier(Modifier::BOLD);
        let current_plus_style = plus_style.add_modifier(Modifier::BOLD);
        let current_minus_style = minus_style.add_modifier(Modifier::BOLD);
        let current_gutter_style = gutter_style.add_modifier(Modifier::BOLD);
        let current_separator_style = separator_style;

        surface.clear_with(area, block_style);
        Widget::render(
            Block::bordered().title(self.data.title.as_str()),
            area,
            surface,
        );

        let inner = area.inner(Margin::all(1));
        if inner.height == 0 {
            return;
        }

        let title_height = inner.height.min(2);
        self.render_title(inner.with_height(title_height), surface, cx);

        let rows_area = inner.clip_top(title_height);
        let viewport_rows = rows_area.height as usize;
        if viewport_rows == 0 {
            return;
        }
        self.scroll = self.scroll.min(self.max_scroll(viewport_rows));
        let current_hunk_bounds = self.current_hunk_bounds();
        self.render_column_headers(rows_area.with_height(1), surface, cx);

        for (idx, row) in self
            .data
            .rows
            .iter()
            .skip(self.scroll)
            .take(viewport_rows.saturating_sub(1))
            .enumerate()
        {
            let row_index = self.scroll + idx;
            let in_current_hunk = current_hunk_bounds
                .map(|(start, end)| row_index >= start && row_index < end)
                .unwrap_or(false);
            let line_area = Rect::new(
                rows_area.x,
                rows_area.y + idx as u16 + 1,
                rows_area.width,
                1,
            );
            match row {
                DiffRow::FileHeader {
                    path,
                    added,
                    removed,
                } => {
                    let style = if in_current_hunk {
                        current_header_style
                    } else {
                        header_style
                    };
                    let text = format!(" {path}   +{added}  -{removed}   side-by-side");
                    surface.set_style(line_area, block_style);
                    surface.set_stringn(
                        line_area.x,
                        line_area.y,
                        text,
                        line_area.width as usize,
                        style,
                    );
                }
                DiffRow::HunkHeader { text } => {
                    let style = if in_current_hunk {
                        current_header_style
                    } else {
                        header_style
                    };
                    surface.set_style(line_area, block_style);
                    surface.set_stringn(
                        line_area.x,
                        line_area.y,
                        format!(" {text}"),
                        line_area.width as usize,
                        style,
                    );
                }
                DiffRow::Context {
                    left_no,
                    right_no,
                    left,
                    right,
                } => self.render_split_row(
                    line_area,
                    surface,
                    *left_no,
                    *right_no,
                    left,
                    right,
                    if in_current_hunk {
                        current_context_style
                    } else {
                        context_style
                    },
                    if in_current_hunk {
                        current_context_style
                    } else {
                        context_style
                    },
                    if in_current_hunk {
                        current_gutter_style
                    } else {
                        gutter_style
                    },
                    if in_current_hunk {
                        current_separator_style
                    } else {
                        separator_style
                    },
                    None,
                ),
                DiffRow::Delete { left_no, left } => self.render_split_row(
                    line_area,
                    surface,
                    Some(*left_no),
                    None,
                    left,
                    "",
                    if in_current_hunk {
                        current_minus_style
                    } else {
                        minus_style
                    },
                    if in_current_hunk {
                        current_context_style
                    } else {
                        context_style
                    },
                    if in_current_hunk {
                        current_gutter_style
                    } else {
                        gutter_style
                    },
                    if in_current_hunk {
                        current_separator_style
                    } else {
                        separator_style
                    },
                    None,
                ),
                DiffRow::Insert { right_no, right } => self.render_split_row(
                    line_area,
                    surface,
                    None,
                    Some(*right_no),
                    "",
                    right,
                    if in_current_hunk {
                        current_context_style
                    } else {
                        context_style
                    },
                    if in_current_hunk {
                        current_plus_style
                    } else {
                        plus_style
                    },
                    if in_current_hunk {
                        current_gutter_style
                    } else {
                        gutter_style
                    },
                    if in_current_hunk {
                        current_separator_style
                    } else {
                        separator_style
                    },
                    None,
                ),
                DiffRow::Modify {
                    left_no,
                    right_no,
                    left,
                    right,
                } => self.render_split_row(
                    line_area,
                    surface,
                    Some(*left_no),
                    Some(*right_no),
                    left,
                    right,
                    if in_current_hunk {
                        current_minus_style
                    } else {
                        minus_style
                    },
                    if in_current_hunk {
                        current_plus_style
                    } else {
                        plus_style
                    },
                    if in_current_hunk {
                        current_gutter_style
                    } else {
                        gutter_style
                    },
                    if in_current_hunk {
                        current_separator_style
                    } else {
                        separator_style
                    },
                    Some((left, right)),
                ),
                DiffRow::Spacer => {
                    let style = if in_current_hunk {
                        subtle_style.add_modifier(Modifier::BOLD)
                    } else {
                        subtle_style
                    };
                    surface.set_style(line_area, block_style);
                    let fill = "─ ─ ─ ─ ─ ─ ─ ─ ─ ─";
                    surface.set_stringn(
                        line_area.x,
                        line_area.y,
                        fill,
                        line_area.width as usize,
                        style,
                    );
                }
            }
        }
    }

    fn required_size(&mut self, viewport: (u16, u16)) -> Option<(u16, u16)> {
        Some(viewport)
    }

    fn id(&self) -> Option<&'static str> {
        Some(Self::ID)
    }
}
