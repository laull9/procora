//! 单服务页内嵌配置编辑器的能力判断与打开逻辑。

use std::path::Path;

use crate::config::ConfigFormat;

use super::{App, ConfigEditor, LiveSession};

/// 单轮日志追赶最多读取的分片数，避免积压日志饿死输入循环。
const LOG_CATCH_UP_BATCHES: usize = 16;

/// 判断当前会话是否具有声明式本地配置和控制能力。
pub(super) fn available(path: Option<&Path>, control_allowed: bool) -> bool {
    control_allowed && path.and_then(ConfigFormat::from_path).is_some()
}

/// 初始化服务页的编辑能力，并按导航来源决定是否立即打开。
pub(super) fn initial(
    path: Option<&Path>,
    control_allowed: bool,
    start_in_editor: bool,
    app: &mut App,
) -> Option<ConfigEditor> {
    app.set_config_edit_allowed(available(path, control_allowed));
    start_in_editor.then(|| open(path, app)).flatten()
}

/// 打开服务配置编辑器，并把可恢复错误留在原服务页展示。
pub(super) fn open(path: Option<&Path>, app: &mut App) -> Option<ConfigEditor> {
    let Some(path) = path else {
        app.set_feedback("当前服务没有可编辑的本地配置入口");
        return None;
    };
    match ConfigEditor::open(path.to_path_buf()) {
        Ok(mut editor) => {
            editor.mark_live_management();
            Some(editor)
        }
        Err(error) => {
            app.set_feedback(format!("无法打开配置编辑器：{error}"));
            None
        }
    }
}

/// 把一次成功写盘继续提交给 Center，并把失败保留为可重试编辑状态。
pub(super) fn apply_saved(
    editor: &mut ConfigEditor,
    app: &mut App,
    session: &mut dyn LiveSession,
) -> bool {
    if !editor.take_saved() {
        return false;
    }
    let close_after_apply = editor.should_quit();
    match session.apply_saved_config() {
        Ok(snapshot) => {
            app.replace_snapshot(snapshot);
            editor.set_message("已保存并应用；服务状态会继续实时刷新，Esc 返回服务页");
            if close_after_apply {
                app.set_feedback("配置已保存并应用");
            }
        }
        Err(error) => editor.keep_open_with_message(format!("已保存，但配置应用失败：{error}")),
    }
    true
}

/// 在编辑器确认无未保存修改后返回服务页。
pub(super) fn close_if_requested(editor: &mut Option<ConfigEditor>) -> bool {
    editor.take_if(|editor| editor.should_quit()).is_some()
}

/// 拉取后台快照；编辑页可见时只更新隐藏状态，不触发无意义重绘。
pub(super) fn poll_snapshot(
    app: &mut App,
    session: &mut dyn LiveSession,
    editor_visible: bool,
) -> bool {
    match session.poll_snapshot() {
        Ok(Some(snapshot)) => {
            let changed = app.replace_snapshot(snapshot);
            !editor_visible && changed
        }
        Ok(None) => false,
        Err(error) => {
            let changed = app.set_feedback(format!("连接异常：{error}"));
            !editor_visible && changed
        }
    }
}

/// 续读当前 Task 的有界日志积压并合并为一次视图更新。
pub(super) fn poll_log(app: &mut App, session: &mut dyn LiveSession) -> bool {
    let Some(task_id) = app.selected_task().map(|task| task.task_id.clone()) else {
        return false;
    };
    let mut bytes = Vec::new();
    let mut gap = false;
    let mut changed = false;
    for _ in 0..LOG_CATCH_UP_BATCHES {
        match session.poll_log(&task_id) {
            Ok(Some(update)) => {
                bytes.extend_from_slice(&update.bytes);
                gap |= update.gap;
            }
            Ok(None) => break,
            Err(error) => {
                changed |= app.set_feedback(format!("日志读取异常：{error}"));
                break;
            }
        }
    }
    changed |= app.append_log(task_id, &bytes, gap);
    changed
}
