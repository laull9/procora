//! 映射字段的键值表编辑状态与按键行为。

use std::collections::BTreeMap;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// 键值表中的一个可编辑行。
#[derive(Clone, Debug)]
pub(super) struct MapRow {
    pub(super) key: String,
    pub(super) value: String,
}

/// 键值表当前编辑的列。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum MapColumn {
    Key,
    Value,
}

/// 单个字符串映射字段的表格编辑状态。
#[derive(Clone, Debug)]
pub(super) struct MapEditor {
    field: usize,
    rows: Vec<MapRow>,
    selected: usize,
    column: MapColumn,
    cursor: usize,
}

impl MapEditor {
    /// 从当前字段映射创建键值表。
    pub(super) fn new(field: usize, values: BTreeMap<String, String>) -> Self {
        let mut rows = values
            .into_iter()
            .map(|(key, value)| MapRow { key, value })
            .collect::<Vec<_>>();
        if rows.is_empty() {
            rows.push(MapRow {
                key: String::new(),
                value: String::new(),
            });
        }
        let cursor = rows[0].key.chars().count();
        Self {
            field,
            rows,
            selected: 0,
            column: MapColumn::Key,
            cursor,
        }
    }

    /// 返回原弹窗中的字段序号。
    pub(super) const fn field(&self) -> usize {
        self.field
    }

    /// 返回当前全部行。
    pub(super) fn rows(&self) -> &[MapRow] {
        &self.rows
    }

    /// 返回当前行、列和字符光标。
    pub(super) const fn position(&self) -> (usize, MapColumn, usize) {
        (self.selected, self.column, self.cursor)
    }

    /// 处理一次键值表输入。
    pub(super) fn handle_key(&mut self, key: KeyEvent) {
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            match key.code {
                KeyCode::Char('n') => self.add_row(),
                KeyCode::Char('d') => self.remove_row(),
                _ => {}
            }
            return;
        }
        match key.code {
            KeyCode::Up => self.move_row(false),
            KeyCode::Down => self.move_row(true),
            KeyCode::Tab | KeyCode::BackTab => self.switch_column(),
            KeyCode::Left => self.cursor = self.cursor.saturating_sub(1),
            KeyCode::Right => {
                self.cursor = (self.cursor + 1).min(self.current().chars().count());
            }
            KeyCode::Home => self.cursor = 0,
            KeyCode::End => self.cursor = self.current().chars().count(),
            KeyCode::Backspace => self.backspace(),
            KeyCode::Delete => self.delete(),
            KeyCode::Char(character) => self.insert(character),
            _ => {}
        }
    }

    /// 校验行并转换为稳定排序的映射。
    pub(super) fn values(&self) -> Result<BTreeMap<String, String>, String> {
        let mut values = BTreeMap::new();
        for row in &self.rows {
            if row.key.is_empty() && row.value.is_empty() {
                continue;
            }
            let key = row.key.trim();
            if key.is_empty() {
                return Err("键值表中的键不能为空".to_owned());
            }
            if values.insert(key.to_owned(), row.value.clone()).is_some() {
                return Err(format!("键值表中的键 `{key}` 重复"));
            }
        }
        Ok(values)
    }

    /// 返回当前单元格文本。
    fn current(&self) -> &str {
        let row = &self.rows[self.selected];
        match self.column {
            MapColumn::Key => &row.key,
            MapColumn::Value => &row.value,
        }
    }

    /// 返回当前单元格可变文本。
    fn current_mut(&mut self) -> &mut String {
        let row = &mut self.rows[self.selected];
        match self.column {
            MapColumn::Key => &mut row.key,
            MapColumn::Value => &mut row.value,
        }
    }

    /// 上下移动当前行。
    fn move_row(&mut self, forward: bool) {
        self.selected = if forward {
            (self.selected + 1).min(self.rows.len() - 1)
        } else {
            self.selected.saturating_sub(1)
        };
        self.cursor = self.current().chars().count();
    }

    /// 在键和值两列间切换。
    fn switch_column(&mut self) {
        self.column = match self.column {
            MapColumn::Key => MapColumn::Value,
            MapColumn::Value => MapColumn::Key,
        };
        self.cursor = self.current().chars().count();
    }

    /// 新增空白行并立即聚焦键列。
    fn add_row(&mut self) {
        self.rows.insert(
            self.selected + 1,
            MapRow {
                key: String::new(),
                value: String::new(),
            },
        );
        self.selected += 1;
        self.column = MapColumn::Key;
        self.cursor = 0;
    }

    /// 删除当前行，同时始终保留一行可输入区域。
    fn remove_row(&mut self) {
        self.rows.remove(self.selected);
        if self.rows.is_empty() {
            self.rows.push(MapRow {
                key: String::new(),
                value: String::new(),
            });
        }
        self.selected = self.selected.min(self.rows.len() - 1);
        self.cursor = self.current().chars().count();
    }

    /// 在当前字符光标处插入文本。
    fn insert(&mut self, character: char) {
        let byte = char_to_byte(self.current(), self.cursor);
        self.current_mut().insert(byte, character);
        self.cursor += 1;
    }

    /// 删除当前光标前一个字符。
    fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let start = char_to_byte(self.current(), self.cursor - 1);
        let end = char_to_byte(self.current(), self.cursor);
        self.current_mut().replace_range(start..end, "");
        self.cursor -= 1;
    }

    /// 删除当前光标处字符。
    fn delete(&mut self) {
        if self.cursor >= self.current().chars().count() {
            return;
        }
        let start = char_to_byte(self.current(), self.cursor);
        let end = char_to_byte(self.current(), self.cursor + 1);
        self.current_mut().replace_range(start..end, "");
    }
}

/// 把字符序号转换为 UTF-8 字节位置。
fn char_to_byte(value: &str, index: usize) -> usize {
    value
        .char_indices()
        .nth(index)
        .map_or(value.len(), |(byte, _)| byte)
}
