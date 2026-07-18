use std::{fs, io, path::PathBuf, time::Duration};

use crate::config::{ConfigFormat, load_path_capture, load_path_text, load_str};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Frame;

use super::{
    config_form::FormConfig,
    config_form_state::{FormEvent, FormState},
    config_ui,
};

/// 配置编辑器的当前输入模式。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum EditorMode {
    /// 以表单、选择器和弹窗编辑常用配置。
    Form,
    /// 直接编辑完整原始配置文本。
    Text,
}

/// 编辑器当前处理独立文件还是需要保留来源的多文件入口。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum EditorSource {
    /// 单个完整配置文件，可以安全转换为结构化表单。
    Standalone,
    /// 多文件入口只允许文本编辑，以免展开后丢失来源。
    DefinitionClosure,
}

/// 配置编辑页的文本、光标、校验和退出状态。
#[derive(Clone, Debug)]
pub struct ConfigEditor {
    path: PathBuf,
    format: ConfigFormat,
    lines: Vec<Vec<char>>,
    row: usize,
    column: usize,
    scroll: usize,
    horizontal_scroll: usize,
    dirty: bool,
    should_quit: bool,
    confirm_discard: bool,
    message: String,
    mode: EditorMode,
    form: Option<FormState>,
    source: EditorSource,
}

impl ConfigEditor {
    /// 从已有配置文件创建编辑页。
    ///
    /// # Errors
    ///
    /// 当扩展名不受支持或文件无法读取时返回错误。
    pub fn open(path: impl Into<PathBuf>) -> io::Result<Self> {
        let path = path.into();
        let format = ConfigFormat::from_path(&path).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "配置扩展名必须是 yaml/yml/toml/json",
            )
        })?;
        let input = fs::read_to_string(&path)?;
        let mut editor = Self::from_text(path, format, &input);
        let capture = load_path_capture(&editor.path);
        if capture.definition_documents > 1 {
            editor.source = EditorSource::DefinitionClosure;
        }
        editor.activate_form();
        Ok(editor)
    }

    /// 从内存文本创建可测试的编辑页状态。
    pub fn from_text(path: impl Into<PathBuf>, format: ConfigFormat, input: &str) -> Self {
        let mut lines = input
            .split('\n')
            .map(|line| line.chars().collect::<Vec<_>>())
            .collect::<Vec<_>>();
        if input.ends_with('\n') && lines.last().is_some_and(Vec::is_empty) {
            lines.pop();
        }
        if lines.is_empty() {
            lines.push(Vec::new());
        }
        Self {
            path: path.into(),
            format,
            lines,
            row: 0,
            column: 0,
            scroll: 0,
            horizontal_scroll: 0,
            dirty: false,
            should_quit: false,
            confirm_discard: false,
            message: "编辑后按 Ctrl-S 校验并保存".to_owned(),
            mode: EditorMode::Text,
            form: None,
            source: EditorSource::Standalone,
        }
    }

    /// 绘制配置编辑器。
    pub fn render(&self, frame: &mut Frame<'_>) {
        config_ui::render(frame, self);
    }

    /// 处理一次终端按键。
    pub fn handle_key(&mut self, key: KeyEvent) {
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            match key.code {
                KeyCode::Char('s') => {
                    self.save();
                    return;
                }
                KeyCode::Char('c') => {
                    self.request_quit();
                    return;
                }
                KeyCode::Char('n' | 'd')
                    if self.mode == EditorMode::Form
                        && self.form.as_ref().is_some_and(FormState::has_map_editor) => {}
                _ => return,
            }
        }
        if key.code == KeyCode::F(1) {
            self.activate_form();
            return;
        }
        if key.code == KeyCode::F(2) {
            self.mode = EditorMode::Text;
            self.form = None;
            "已进入高级文本模式；F1 返回表单".clone_into(&mut self.message);
            return;
        }
        if key.code != KeyCode::Esc {
            self.confirm_discard = false;
        }
        if self.mode == EditorMode::Form {
            if key.code == KeyCode::Esc
                && self
                    .form
                    .as_ref()
                    .is_some_and(|form| form.dialog().is_none() && !form.has_pending_delete())
            {
                self.request_quit();
                return;
            }
            if let Some(form) = &mut self.form {
                match form.handle_key(key) {
                    FormEvent::None => {}
                    FormEvent::Changed => self.synchronize_form(false),
                    FormEvent::Reload => self.synchronize_form(true),
                    FormEvent::Message(message) => self.message = message,
                }
            }
            return;
        }
        match key.code {
            KeyCode::Esc => self.request_quit(),
            KeyCode::Char(character) => self.insert(character),
            KeyCode::Enter => self.newline(),
            KeyCode::Backspace => self.backspace(),
            KeyCode::Delete => self.delete(),
            KeyCode::Left => self.left(),
            KeyCode::Right => self.right(),
            KeyCode::Up => self.up(),
            KeyCode::Down => self.down(),
            KeyCode::Home => self.column = 0,
            KeyCode::End => self.column = self.lines[self.row].len(),
            KeyCode::Tab => {
                for _ in 0..2 {
                    self.insert(' ');
                }
            }
            _ => {}
        }
    }

    /// 返回编辑页是否请求退出。
    pub const fn should_quit(&self) -> bool {
        self.should_quit
    }

    /// 返回当前缓冲区的完整配置文本。
    pub fn text(&self) -> String {
        let mut text = self
            .lines
            .iter()
            .map(|line| line.iter().collect::<String>())
            .collect::<Vec<_>>()
            .join("\n");
        text.push('\n');
        text
    }

    /// 返回当前状态提示。
    pub fn message(&self) -> &str {
        &self.message
    }

    /// 返回当前配置文件路径。
    pub fn path(&self) -> &std::path::Path {
        &self.path
    }

    /// 返回高级文本模式当前使用的配置格式。
    pub(crate) const fn format(&self) -> ConfigFormat {
        self.format
    }

    /// 返回当前光标位置。
    pub const fn cursor(&self) -> (usize, usize) {
        (self.row, self.column)
    }

    /// 返回首个可见行号。
    pub const fn scroll(&self) -> usize {
        self.scroll
    }

    /// 返回高级文本模式首个可见显示列。
    pub const fn horizontal_scroll(&self) -> usize {
        self.horizontal_scroll
    }

    /// 推进一次结构化表单的全局折叠文本自动滚动。
    pub fn advance_auto_scroll(&mut self, elapsed: Duration) -> bool {
        self.form
            .as_mut()
            .filter(|_| self.mode == EditorMode::Form)
            .is_some_and(|form| form.advance_auto_scroll(elapsed))
    }

    /// 更新首个可见行以保持光标在编辑区域内。
    pub fn ensure_visible(&mut self, height: usize) {
        if self.row < self.scroll {
            self.scroll = self.row;
        } else if self.row >= self.scroll + height.max(1) {
            self.scroll = self.row + 1 - height.max(1);
        }
    }

    /// 更新首个可见显示列以保持文本光标始终可见。
    pub fn ensure_horizontal_visible(&mut self, width: usize) {
        let display_column = self.lines[self.row]
            .iter()
            .take(self.column)
            .map(|character| super::text_view::width(&character.to_string()))
            .sum::<usize>();
        if display_column < self.horizontal_scroll {
            self.horizontal_scroll = display_column;
        } else if display_column >= self.horizontal_scroll + width.max(1) {
            self.horizontal_scroll = display_column + 1 - width.max(1);
        }
    }

    /// 返回用于渲染的文本行。
    pub fn lines(&self) -> impl Iterator<Item = String> + '_ {
        self.lines.iter().map(|line| line.iter().collect())
    }

    /// 返回当前是否处于结构化表单模式。
    pub(crate) fn is_form_mode(&self) -> bool {
        self.mode == EditorMode::Form
    }

    /// 返回当前表单状态，文本模式或无效配置时为空。
    pub(crate) fn form(&self) -> Option<&FormState> {
        self.form.as_ref()
    }

    /// 校验并原子语义保存当前缓冲区。
    fn save(&mut self) {
        if self.mode == EditorMode::Form {
            self.synchronize_form(false);
            if self.message.starts_with("配置无效") || self.message.starts_with("表单输出失败")
            {
                return;
            }
        }
        let text = self.text();
        match load_path_text(&self.path, &text) {
            Ok(compiled) => match fs::write(&self.path, text) {
                Ok(()) => {
                    self.dirty = false;
                    self.message = format!(
                        "已保存：{} 个任务，{} 个管理依赖",
                        compiled.spec.tasks.len(),
                        compiled.dependencies.len()
                    );
                }
                Err(error) => self.message = format!("保存失败：{error}"),
            },
            Err(error) => self.message = format!("配置无效：{error}"),
        }
    }

    /// 处理带未保存确认的退出请求。
    fn request_quit(&mut self) {
        if self.dirty && !self.confirm_discard {
            self.confirm_discard = true;
            "有未保存修改；再次按 Esc 或 Ctrl-C 放弃".clone_into(&mut self.message);
        } else {
            self.should_quit = true;
        }
    }

    /// 在光标处插入字符。
    fn insert(&mut self, character: char) {
        self.lines[self.row].insert(self.column, character);
        self.column += 1;
        self.changed();
    }

    /// 在光标处拆分当前行。
    fn newline(&mut self) {
        let tail = self.lines[self.row].split_off(self.column);
        self.row += 1;
        self.column = 0;
        self.lines.insert(self.row, tail);
        self.changed();
    }

    /// 删除光标之前的字符或合并上一行。
    fn backspace(&mut self) {
        if self.column > 0 {
            self.column -= 1;
            self.lines[self.row].remove(self.column);
        } else if self.row > 0 {
            let current = self.lines.remove(self.row);
            self.row -= 1;
            self.column = self.lines[self.row].len();
            self.lines[self.row].extend(current);
        } else {
            return;
        }
        self.changed();
    }

    /// 删除光标处字符或合并下一行。
    fn delete(&mut self) {
        if self.column < self.lines[self.row].len() {
            self.lines[self.row].remove(self.column);
        } else if self.row + 1 < self.lines.len() {
            let next = self.lines.remove(self.row + 1);
            self.lines[self.row].extend(next);
        } else {
            return;
        }
        self.changed();
    }

    /// 左移光标并允许跨行。
    fn left(&mut self) {
        if self.column > 0 {
            self.column -= 1;
        } else if self.row > 0 {
            self.row -= 1;
            self.column = self.lines[self.row].len();
        }
    }

    /// 右移光标并允许跨行。
    fn right(&mut self) {
        if self.column < self.lines[self.row].len() {
            self.column += 1;
        } else if self.row + 1 < self.lines.len() {
            self.row += 1;
            self.column = 0;
        }
    }

    /// 上移光标。
    fn up(&mut self) {
        if self.row > 0 {
            self.row -= 1;
            self.column = self.column.min(self.lines[self.row].len());
        }
    }

    /// 下移光标。
    fn down(&mut self) {
        if self.row + 1 < self.lines.len() {
            self.row += 1;
            self.column = self.column.min(self.lines[self.row].len());
        }
    }

    /// 标记缓冲区已修改并清除旧反馈。
    fn changed(&mut self) {
        self.dirty = true;
        self.confirm_discard = false;
        "未保存；Ctrl-S 校验并保存".clone_into(&mut self.message);
    }

    /// 将当前有效文本配置转换为结构化表单，失败时继续保留高级文本模式。
    fn activate_form(&mut self) {
        if self.source == EditorSource::DefinitionClosure {
            self.mode = EditorMode::Text;
            self.form = None;
            "多文件配置保持高级文本模式，避免表单展开后丢失输入来源".clone_into(&mut self.message);
            return;
        }
        let text = self.text();
        let compiled = if self.path.exists() {
            load_path_text(&self.path, &text)
        } else {
            load_str(&text, self.format)
        };
        match compiled {
            Ok(compiled) => {
                self.form = Some(FormState::new(FormConfig::from_compiled(
                    compiled,
                    self.path.parent(),
                )));
                self.mode = EditorMode::Form;
                "表单模式：Enter 编辑，h 健康检查，n 新建，d 删除，F3 自动滚动，F2 高级文本"
                    .clone_into(&mut self.message);
            }
            Err(error) => {
                self.mode = EditorMode::Text;
                self.form = None;
                self.message = format!("配置无效，无法打开表单：{error}；请在文本模式修复");
            }
        }
    }

    /// 把表单模型转为目标格式文本，并阻止不符合完整配置规则的改动落盘。
    fn synchronize_form(&mut self, reload: bool) {
        let Some(form) = &self.form else {
            return;
        };
        let text = match form.config().text(self.format) {
            Ok(text) => text,
            Err(error) => {
                self.message = format!("表单输出失败：{error}");
                return;
            }
        };
        let compiled = if self.path.exists() {
            load_path_text(&self.path, &text)
        } else {
            load_str(&text, self.format)
        };
        match compiled {
            Ok(compiled) => {
                self.lines = text
                    .trim_end_matches('\n')
                    .split('\n')
                    .map(|line| line.chars().collect())
                    .collect();
                if self.lines.is_empty() {
                    self.lines.push(Vec::new());
                }
                self.row = 0;
                self.column = 0;
                self.scroll = 0;
                self.horizontal_scroll = 0;
                self.changed();
                if reload {
                    let profile = compiled.active_profile.clone();
                    self.form = Some(FormState::new(FormConfig::from_compiled(
                        compiled,
                        self.path.parent(),
                    )));
                    self.message = profile.map_or_else(
                        || "已切换到基础配置预览；Ctrl-S 保存".to_owned(),
                        |name| format!("已切换到 profile `{name}` 预览；Ctrl-S 保存"),
                    );
                }
            }
            Err(error) => self.message = format!("配置无效：{error}"),
        }
    }
}
