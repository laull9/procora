use crossterm::event::KeyEvent;

use super::{
    config_editor::ConfigEditor,
    selection::{SelectionEvent, SelectionItem, SelectionState},
};

/// 整个配置编辑页退出确认提供的动作。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum EditorExitChoice {
    Save,
    Discard,
    Cancel,
}

impl ConfigEditor {
    /// 返回整个编辑页的未保存退出选择栏。
    pub(super) fn exit_prompt(&self) -> Option<&SelectionState<EditorExitChoice>> {
        self.exit_prompt.as_ref()
    }

    /// 处理带未保存确认的退出请求。
    pub(super) fn request_quit(&mut self) {
        if self.has_unsaved_changes() {
            self.exit_prompt = Some(editor_exit_selection());
            "配置有未保存修改，请选择处理方式".clone_into(&mut self.message);
        } else {
            self.should_quit = true;
        }
    }

    /// 处理整个配置编辑页的保存、放弃和取消退出选择。
    pub(super) fn handle_exit_prompt(&mut self, key: KeyEvent) {
        let prompt = self.exit_prompt.as_mut().expect("编辑页退出选择状态存在");
        match prompt.handle_key(key) {
            SelectionEvent::Pending => {}
            SelectionEvent::Cancelled | SelectionEvent::Selected(EditorExitChoice::Cancel) => {
                self.exit_prompt = None;
                "继续编辑".clone_into(&mut self.message);
            }
            SelectionEvent::Selected(EditorExitChoice::Discard) => {
                self.should_quit = true;
            }
            SelectionEvent::Selected(EditorExitChoice::Save) => {
                self.exit_prompt = None;
                self.save_requested();
                if !self.dirty {
                    self.should_quit = true;
                }
            }
        }
    }
}

/// 创建整个配置编辑页通用的未保存退出选择栏。
fn editor_exit_selection() -> SelectionState<EditorExitChoice> {
    SelectionState::new(vec![
        SelectionItem::new("保存并退出", "校验并写入配置文件", EditorExitChoice::Save),
        SelectionItem::new(
            "不保存退出",
            "放弃尚未写入的修改",
            EditorExitChoice::Discard,
        ),
        SelectionItem::new("取消", "返回配置编辑页", EditorExitChoice::Cancel),
    ])
}
