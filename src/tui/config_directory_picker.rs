use std::{
    cmp::Ordering,
    fs,
    path::{Component, Path, PathBuf},
};

use crossterm::event::{KeyCode, KeyEvent};

use super::{config_form_dialog::Dialog, config_form_field::DialogFieldKind};

/// 目录浏览器中的一条可执行项目。
#[derive(Clone, Debug)]
struct DirectoryEntry {
    label: String,
    action: EntryAction,
}

/// 目录项目被确认时执行的动作。
#[derive(Clone, Debug)]
enum EntryAction {
    SelectCurrent,
    Parent,
    Open(PathBuf),
}

/// 终端内跨平台目录浏览器状态。
#[derive(Clone, Debug)]
pub(super) struct DirectoryPicker {
    field: usize,
    base_directory: PathBuf,
    current: Option<PathBuf>,
    entries: Vec<DirectoryEntry>,
    selected: usize,
    error: Option<String>,
}

/// 目录浏览器按键处理后的结果。
pub(super) enum DirectoryPickerEvent {
    Continue,
    Cancel,
    Selected { field: usize, value: String },
}

impl DirectoryPicker {
    /// 从字段现有值或配置目录打开浏览器。
    pub(super) fn new(field: usize, configured: &str, base_directory: &Path) -> Self {
        let base_directory = stable_absolute_path(base_directory);
        let requested = if configured.trim().is_empty() {
            base_directory.clone()
        } else {
            absolute_path(Path::new(configured), &base_directory)
        };
        let current = nearest_directory(&requested).unwrap_or_else(|| base_directory.clone());
        let mut picker = Self {
            field,
            base_directory,
            current: Some(current),
            entries: Vec::new(),
            selected: 0,
            error: None,
        };
        picker.refresh();
        picker
    }

    /// 返回当前目录标题；Windows 盘符视图没有具体目录。
    pub(super) fn location(&self) -> String {
        self.current.as_ref().map_or_else(
            || "可用驱动器".to_owned(),
            |path| path.display().to_string(),
        )
    }

    /// 返回当前可见项目。
    pub(super) fn entries(&self) -> impl Iterator<Item = (&str, bool)> {
        self.entries
            .iter()
            .enumerate()
            .map(|(index, entry)| (entry.label.as_str(), index == self.selected))
    }

    /// 返回当前选中序号。
    pub(super) const fn selected(&self) -> usize {
        self.selected
    }

    /// 返回最近一次目录读取错误。
    pub(super) fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// 处理导航、确认或取消按键。
    pub(super) fn handle_key(&mut self, key: KeyEvent) -> DirectoryPickerEvent {
        match key.code {
            KeyCode::Esc => DirectoryPickerEvent::Cancel,
            KeyCode::Up | KeyCode::Char('k') => {
                self.selected = self.selected.saturating_sub(1);
                DirectoryPickerEvent::Continue
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.selected = (self.selected + 1).min(self.entries.len().saturating_sub(1));
                DirectoryPickerEvent::Continue
            }
            KeyCode::Home => {
                self.selected = 0;
                DirectoryPickerEvent::Continue
            }
            KeyCode::End => {
                self.selected = self.entries.len().saturating_sub(1);
                DirectoryPickerEvent::Continue
            }
            KeyCode::Left | KeyCode::Backspace => {
                self.open_parent();
                DirectoryPickerEvent::Continue
            }
            KeyCode::Char('r') => {
                self.refresh();
                DirectoryPickerEvent::Continue
            }
            KeyCode::Char(' ') => self.select_current(),
            KeyCode::Enter | KeyCode::Right => self.activate(),
            _ => DirectoryPickerEvent::Continue,
        }
    }

    /// 读取当前目录并稳定排序子目录。
    fn refresh(&mut self) {
        self.entries.clear();
        self.error = None;
        let Some(current) = self.current.clone() else {
            self.refresh_windows_drives();
            return;
        };
        self.entries.push(DirectoryEntry {
            label: "✓ 选择此目录".to_owned(),
            action: EntryAction::SelectCurrent,
        });
        if current.parent().is_some() || cfg!(windows) {
            self.entries.push(DirectoryEntry {
                label: "↰ 返回上级".to_owned(),
                action: EntryAction::Parent,
            });
        }
        match fs::read_dir(&current) {
            Ok(entries) => {
                let mut directories = entries
                    .filter_map(Result::ok)
                    .filter(|entry| entry.path().is_dir())
                    .map(|entry| DirectoryEntry {
                        label: format!("▸ {}/", entry.file_name().to_string_lossy()),
                        action: EntryAction::Open(entry.path()),
                    })
                    .collect::<Vec<_>>();
                directories.sort_by(|left, right| compare_names(&left.label, &right.label));
                self.entries.extend(directories);
            }
            Err(error) => self.error = Some(format!("无法读取目录：{error}")),
        }
        self.selected = self.selected.min(self.entries.len().saturating_sub(1));
    }

    /// Windows 根目录之外提供独立盘符列表；其他平台不会进入该状态。
    fn refresh_windows_drives(&mut self) {
        #[cfg(windows)]
        {
            for letter in b'A'..=b'Z' {
                let path = PathBuf::from(format!("{}:\\", char::from(letter)));
                if path.is_dir() {
                    self.entries.push(DirectoryEntry {
                        label: format!("▸ {}", path.display()),
                        action: EntryAction::Open(path),
                    });
                }
            }
            if self.entries.is_empty() {
                self.error = Some("没有检测到可访问的驱动器".to_owned());
            }
        }
        #[cfg(not(windows))]
        {
            self.error = Some("当前平台没有可用的目录根视图".to_owned());
        }
        self.selected = self.selected.min(self.entries.len().saturating_sub(1));
    }

    /// 激活当前项目，选择当前目录或进入目标目录。
    fn activate(&mut self) -> DirectoryPickerEvent {
        let Some(action) = self
            .entries
            .get(self.selected)
            .map(|entry| entry.action.clone())
        else {
            return DirectoryPickerEvent::Continue;
        };
        match action {
            EntryAction::SelectCurrent => self.select_current(),
            EntryAction::Parent => {
                self.open_parent();
                DirectoryPickerEvent::Continue
            }
            EntryAction::Open(path) => {
                self.current = Some(path);
                self.selected = 0;
                self.refresh();
                DirectoryPickerEvent::Continue
            }
        }
    }

    /// 返回上级；Windows 驱动器根会进入盘符列表。
    fn open_parent(&mut self) {
        let Some(current) = self.current.clone() else {
            return;
        };
        if let Some(parent) = current.parent() {
            self.current = Some(parent.to_path_buf());
        } else if cfg!(windows) {
            self.current = None;
        } else {
            return;
        }
        self.selected = 0;
        self.refresh();
    }

    /// 选择当前位置并生成相对配置目录的可移植文本。
    fn select_current(&self) -> DirectoryPickerEvent {
        let Some(current) = self.current.as_deref() else {
            return DirectoryPickerEvent::Continue;
        };
        DirectoryPickerEvent::Selected {
            field: self.field,
            value: portable_path(current, &self.base_directory),
        }
    }
}

impl Dialog {
    /// 返回当前字段是否支持目录浏览。
    pub(crate) fn selected_is_directory(&self) -> bool {
        self.fields[self.selected].kind == DialogFieldKind::Directory
    }

    /// 为当前目录字段打开浏览器。
    pub(crate) fn open_directory_picker(&mut self, base_directory: &Path) -> Result<(), String> {
        if !self.selected_is_directory() {
            return Err("当前字段不是目录".to_owned());
        }
        self.directory_picker = Some(DirectoryPicker::new(
            self.selected,
            &self.fields[self.selected].value,
            base_directory,
        ));
        Ok(())
    }

    /// 处理目录浏览器输入；返回空表示当前没有打开子弹窗。
    pub(crate) fn handle_directory_key(&mut self, key: KeyEvent) -> Option<()> {
        let event = self.directory_picker.as_mut()?.handle_key(key);
        match event {
            DirectoryPickerEvent::Continue => {}
            DirectoryPickerEvent::Cancel => self.directory_picker = None,
            DirectoryPickerEvent::Selected { field, value } => {
                self.fields[field].cursor = value.chars().count();
                self.fields[field].value = value;
                self.directory_picker = None;
            }
        }
        Some(())
    }

    /// 返回当前打开的目录浏览器。
    pub(super) const fn directory_picker(&self) -> Option<&DirectoryPicker> {
        self.directory_picker.as_ref()
    }
}

/// 把配置目录转换为绝对路径，目标暂不可规范化时仍保持可浏览。
fn stable_absolute_path(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| {
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir()
                .map_or_else(|_| path.to_path_buf(), |current| current.join(path))
        }
    })
}

/// 将相对配置值转成用于浏览的绝对路径。
fn absolute_path(path: &Path, base_directory: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        base_directory.join(path)
    }
}

/// 从输入路径向上寻找最近的现有目录。
fn nearest_directory(path: &Path) -> Option<PathBuf> {
    let mut candidate = path.to_path_buf();
    loop {
        if candidate.is_dir() {
            return Some(candidate);
        }
        if !candidate.pop() {
            return None;
        }
    }
}

/// 以不区分 ASCII 大小写的方式稳定排序目录名。
fn compare_names(left: &str, right: &str) -> Ordering {
    left.to_ascii_lowercase()
        .cmp(&right.to_ascii_lowercase())
        .then_with(|| left.cmp(right))
}

/// 尽量生成相对配置目录的路径，不同 Windows 盘符时保留绝对路径。
fn portable_path(path: &Path, base_directory: &Path) -> String {
    let canonical_path = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let canonical_base =
        fs::canonicalize(base_directory).unwrap_or_else(|_| base_directory.to_path_buf());
    let value = relative_path(&canonical_path, &canonical_base).unwrap_or(canonical_path);
    let text = if value.as_os_str().is_empty() {
        ".".to_owned()
    } else {
        value.to_string_lossy().into_owned()
    };
    #[cfg(windows)]
    let text = text.replace('\\', "/");
    text
}

/// 在根前缀兼容时计算从基础目录到目标目录的词法相对路径。
fn relative_path(path: &Path, base: &Path) -> Option<PathBuf> {
    let path_components = path.components().collect::<Vec<_>>();
    let base_components = base.components().collect::<Vec<_>>();
    let common = path_components
        .iter()
        .zip(&base_components)
        .take_while(|(left, right)| left == right)
        .count();
    if common == 0
        || matches!(
            path_components.get(common),
            Some(Component::Prefix(_) | Component::RootDir)
        )
    {
        return None;
    }
    let mut relative = PathBuf::new();
    for component in &base_components[common..] {
        if matches!(component, Component::Normal(_)) {
            relative.push("..");
        }
    }
    for component in &path_components[common..] {
        relative.push(component.as_os_str());
    }
    Some(relative)
}
