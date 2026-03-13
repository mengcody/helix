use helix_core::Position;
use helix_view::{
    editor::Action,
    graphics::{CursorKind, Rect},
    Editor,
};
use std::collections::BTreeSet;
use std::error::Error;
use std::path::{Path, PathBuf};
use tui::buffer::Buffer as Surface;

#[derive(Debug, Clone, PartialEq, Eq)]
struct FileTreeNode {
    path: PathBuf,
    depth: usize,
    is_dir: bool,
}

pub struct FileTreeSidebar {
    root: PathBuf,
    expanded: BTreeSet<PathBuf>,
    nodes: Vec<FileTreeNode>,
    selected: usize,
    scroll: usize,
    width: u16,
    focused: bool,
}

impl FileTreeSidebar {
    pub const DEFAULT_WIDTH: u16 = 30;

    pub fn new(root: PathBuf, editor: &Editor) -> std::io::Result<Self> {
        let root = helix_stdx::path::normalize(&root);
        let mut expanded = BTreeSet::new();
        expanded.insert(root.clone());

        let mut sidebar = Self {
            root,
            expanded,
            nodes: Vec::new(),
            selected: 0,
            scroll: 0,
            width: Self::DEFAULT_WIDTH,
            focused: true,
        };
        sidebar.rebuild(editor, None)?;
        Ok(sidebar)
    }

    pub fn is_focused(&self) -> bool {
        self.focused
    }

    pub fn focus(&mut self) {
        self.focused = true;
    }

    pub fn unfocus(&mut self) {
        self.focused = false;
    }

    pub fn width(&self, available_width: u16) -> u16 {
        self.width.min(available_width.saturating_sub(1)).max(1)
    }

    pub fn move_up(&mut self) {
        if self.selected > 0 {
            self.selected -= 1;
        }
    }

    pub fn move_down(&mut self) {
        if self.selected + 1 < self.nodes.len() {
            self.selected += 1;
        }
    }

    pub fn collapse_or_parent(&mut self, editor: &Editor) -> std::io::Result<()> {
        let Some(node) = self.selected_node().cloned() else {
            return Ok(());
        };

        if node.is_dir && self.expanded.contains(&node.path) {
            self.expanded.remove(&node.path);
            self.rebuild(editor, Some(node.path))?;
            return Ok(());
        }

        if let Some(parent_idx) = self
            .nodes
            .iter()
            .enumerate()
            .take(self.selected)
            .rev()
            .find_map(|(idx, candidate)| {
                (candidate.depth < node.depth && node.path.starts_with(&candidate.path))
                    .then_some(idx)
            })
        {
            self.selected = parent_idx;
        }

        Ok(())
    }

    pub fn expand_or_descend(&mut self, editor: &Editor) -> std::io::Result<()> {
        let Some(node) = self.selected_node().cloned() else {
            return Ok(());
        };

        if !node.is_dir {
            return Ok(());
        }

        if self.expanded.insert(node.path.clone()) {
            self.rebuild(editor, Some(node.path.clone()))?;
            return Ok(());
        }

        if let Some(child_idx) = self
            .nodes
            .iter()
            .enumerate()
            .skip(self.selected + 1)
            .find_map(|(idx, candidate)| {
                (candidate.depth == node.depth + 1 && candidate.path.starts_with(&node.path))
                    .then_some(idx)
            })
        {
            self.selected = child_idx;
        }

        Ok(())
    }

    pub fn activate(&mut self, editor: &mut Editor) -> std::io::Result<()> {
        let Some(node) = self.selected_node().cloned() else {
            return Ok(());
        };

        if node.is_dir {
            if self.expanded.contains(&node.path) {
                self.expanded.remove(&node.path);
            } else {
                self.expanded.insert(node.path.clone());
            }
            self.rebuild(editor, Some(node.path))?;
            return Ok(());
        }

        if let Err(err) = editor.open(&node.path, Action::Replace) {
            let message = err
                .source()
                .map(ToString::to_string)
                .unwrap_or_else(|| format!("unable to open \"{}\"", node.path.display()));
            editor.set_error(message);
        } else {
            self.unfocus();
        }

        Ok(())
    }

    pub fn render(&mut self, area: Rect, surface: &mut Surface, editor: &Editor) {
        let background = editor.theme.get("ui.background");
        let text_style = editor.theme.get("ui.text");
        let selected_style = editor.theme.get("ui.text.focus");
        let directory_style = editor.theme.get("ui.text.directory");
        let border_style = editor.theme.get("ui.window");
        surface.set_style(area, background);

        let height = area.height as usize;
        self.ensure_selection_in_bounds();
        self.ensure_scroll(height);

        for (row, node) in self.nodes.iter().enumerate().skip(self.scroll).take(height) {
            let y = area.y + (row - self.scroll) as u16;
            let style = if row == self.selected {
                selected_style
            } else if node.is_dir {
                directory_style
            } else {
                text_style
            };

            let line = self.format_node(node);
            surface.set_style(Rect::new(area.x, y, area.width, 1), style);
            surface.set_stringn(area.x, y, line, area.width as usize, style);
        }

        let separator_x = area.x + area.width;
        for y in area.top()..area.bottom() {
            surface[(separator_x, y)]
                .set_symbol(tui::symbols::line::VERTICAL)
                .set_style(border_style);
        }
    }

    pub fn cursor(&self, area: Rect) -> (Option<Position>, CursorKind) {
        if !self.focused || area.height == 0 {
            return (None, CursorKind::Hidden);
        }

        let visible_row = self.selected.saturating_sub(self.scroll) as u16;
        if visible_row >= area.height {
            return (None, CursorKind::Hidden);
        }

        (
            Some(Position::new(
                (area.y + visible_row) as usize,
                area.x as usize,
            )),
            CursorKind::Block,
        )
    }

    fn selected_node(&self) -> Option<&FileTreeNode> {
        self.nodes.get(self.selected)
    }

    fn rebuild(&mut self, editor: &Editor, selected_path: Option<PathBuf>) -> std::io::Result<()> {
        let mut nodes = Vec::new();
        self.collect_nodes(editor, &self.root, 0, &mut nodes)?;
        self.nodes = nodes;

        if let Some(path) = selected_path {
            if let Some(index) = self.nodes.iter().position(|node| node.path == path) {
                self.selected = index;
            }
        }

        self.ensure_selection_in_bounds();
        Ok(())
    }

    fn collect_nodes(
        &self,
        editor: &Editor,
        path: &Path,
        depth: usize,
        nodes: &mut Vec<FileTreeNode>,
    ) -> std::io::Result<()> {
        let is_dir = path.is_dir();
        nodes.push(FileTreeNode {
            path: path.to_path_buf(),
            depth,
            is_dir,
        });

        if is_dir && self.expanded.contains(path) {
            for (child_path, child_is_dir) in super::directory_content(path, editor, false)? {
                nodes.push(FileTreeNode {
                    path: child_path.clone(),
                    depth: depth + 1,
                    is_dir: child_is_dir,
                });

                if child_is_dir && self.expanded.contains(&child_path) {
                    self.collect_descendants(editor, &child_path, depth + 2, nodes)?;
                }
            }
        }

        Ok(())
    }

    fn collect_descendants(
        &self,
        editor: &Editor,
        dir: &Path,
        depth: usize,
        nodes: &mut Vec<FileTreeNode>,
    ) -> std::io::Result<()> {
        for (child_path, child_is_dir) in super::directory_content(dir, editor, false)? {
            nodes.push(FileTreeNode {
                path: child_path.clone(),
                depth,
                is_dir: child_is_dir,
            });

            if child_is_dir && self.expanded.contains(&child_path) {
                self.collect_descendants(editor, &child_path, depth + 1, nodes)?;
            }
        }

        Ok(())
    }

    fn ensure_selection_in_bounds(&mut self) {
        if self.nodes.is_empty() {
            self.selected = 0;
            self.scroll = 0;
        } else {
            self.selected = self.selected.min(self.nodes.len() - 1);
        }
    }

    fn ensure_scroll(&mut self, height: usize) {
        if height == 0 {
            self.scroll = 0;
            return;
        }

        if self.selected < self.scroll {
            self.scroll = self.selected;
        } else if self.selected >= self.scroll + height {
            self.scroll = self.selected + 1 - height;
        }
    }

    fn format_node(&self, node: &FileTreeNode) -> String {
        let indent = "  ".repeat(node.depth);
        let icon = if node.is_dir {
            if self.expanded.contains(&node.path) {
                "▾ "
            } else {
                "▸ "
            }
        } else {
            "  "
        };

        let name = if node.path == self.root {
            self.root
                .file_name()
                .filter(|name| !name.is_empty())
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| self.root.display().to_string())
        } else {
            node.path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| node.path.display().to_string())
        };

        format!("{indent}{icon}{name}")
    }
}

#[cfg(test)]
mod tests {
    use super::FileTreeSidebar;

    #[test]
    fn sidebar_width_leaves_room_for_editor() {
        let sidebar = FileTreeSidebar {
            root: Default::default(),
            expanded: Default::default(),
            nodes: Vec::new(),
            selected: 0,
            scroll: 0,
            width: FileTreeSidebar::DEFAULT_WIDTH,
            focused: false,
        };

        assert_eq!(sidebar.width(10), 9);
        assert_eq!(sidebar.width(1), 1);
    }
}
