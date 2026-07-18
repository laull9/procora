//! 配置编辑页的文本操作、校验保存和退出保护测试。

use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use procora::config::ConfigFormat;
use procora::tui::ConfigEditor;
use ratatui::{Terminal, backend::TestBackend, buffer::Cell, style::Color};

/// 同一进程内并行创建临时配置时使用的去重序号。
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// 创建当前测试独占的配置路径。
fn temporary_config() -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "procora-editor-{}-{nonce}-{sequence}.yaml",
        std::process::id()
    ))
}

/// 向编辑器发送一个无修饰键。
fn press(editor: &mut ConfigEditor, code: KeyCode) {
    editor.handle_key(KeyEvent::new(code, KeyModifiers::NONE));
}

/// 向当前表单字段逐字输入文本。
fn type_text(editor: &mut ConfigEditor, value: &str) {
    for character in value.chars() {
        press(editor, KeyCode::Char(character));
    }
}

/// 清空当前字段末尾已知数量的字符。
fn backspace(editor: &mut ConfigEditor, count: usize) {
    for _ in 0..count {
        press(editor, KeyCode::Backspace);
    }
}

#[test]
// 只有有效配置才会保存。
fn only_valid_config_is_saved() {
    let path = temporary_config();
    let initial = "version: 1\nproject: demo\ntasks: {}\n";
    fs::write(&path, initial).unwrap();
    let mut editor = ConfigEditor::from_text(&path, ConfigFormat::Yaml, initial);
    editor.handle_key(KeyEvent::new(KeyCode::Char('!'), KeyModifiers::NONE));
    editor.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));
    assert!(editor.message().starts_with("配置无效"));
    assert_eq!(fs::read_to_string(&path).unwrap(), initial);
    fs::remove_file(path).unwrap();
}

#[test]
// 未保存修改需要二次确认退出。
fn unsaved_changes_require_exit_confirmation() {
    let mut editor = ConfigEditor::from_text(
        "procora.yaml",
        ConfigFormat::Yaml,
        "version: 1\nproject: demo\ntasks: {}\n",
    );
    editor.handle_key(KeyEvent::new(KeyCode::Char('#'), KeyModifiers::NONE));
    editor.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(!editor.should_quit());
    assert!(editor.message().contains("再次按"));
    editor.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(editor.should_quit());
}

#[test]
// 宽屏编辑页会显示配置内容与依赖引导。
fn wide_editor_shows_config_and_dependency_guidance() {
    let editor = ConfigEditor::from_text(
        "procora.yaml",
        ConfigFormat::Yaml,
        "version: 1\nproject: demo\ntasks: {}\n",
    );
    let backend = TestBackend::new(110, 28);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|frame| editor.render(frame)).unwrap();
    let text = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(Cell::symbol)
        .collect::<String>()
        .replace(' ', "");

    assert!(text.contains("Procora配置编辑器"));
    assert!(text.contains("version:1"));
    assert!(text.contains("管理依赖"));
    assert!(text.contains("${dependency.<id>}"));
}

#[test]
// 高级文本模式会按配置语法为不同类型的词元着色。
fn advanced_editor_highlights_config_syntax() {
    for (path, format, input) in [
        (
            "procora.yaml",
            ConfigFormat::Yaml,
            "version: 1\nproject: demo\ntasks: {}",
        ),
        (
            "procora.toml",
            ConfigFormat::Toml,
            "version = 1\nproject = \"demo\"\n[tasks]",
        ),
        ("procora.json", ConfigFormat::Json, r#"{"version":1}"#),
    ] {
        let editor = ConfigEditor::from_text(path, format, input);
        let backend = TestBackend::new(90, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| editor.render(frame)).unwrap();
        let buffer = terminal.backend().buffer();
        let mut colors = Vec::new();
        for y in 4..7 {
            for x in 6..40 {
                let cell = buffer.cell((x, y)).unwrap();
                if !cell.symbol().trim().is_empty()
                    && cell.fg != Color::Reset
                    && !colors.contains(&cell.fg)
                {
                    colors.push(cell.fg);
                }
            }
        }

        assert!(
            colors.len() >= 2,
            "`{path}` 的关键字、字符串和数字应使用不同颜色"
        );
    }
}

#[test]
// 结构化表单右侧的键位提示使用独立强调色。
fn form_key_hints_are_color_highlighted() {
    let path = temporary_config();
    fs::write(&path, "version: 1\nproject: demo\ntasks: {}\n").unwrap();
    let editor = ConfigEditor::open(&path).unwrap();
    let backend = TestBackend::new(110, 28);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|frame| editor.render(frame)).unwrap();
    let buffer = terminal.backend().buffer();
    let mut highlighted_tab = false;
    for y in 0..buffer.area.height {
        for x in 0..buffer.area.width.saturating_sub(2) {
            let first = buffer.cell((x, y)).unwrap();
            let second = buffer.cell((x + 1, y)).unwrap();
            let third = buffer.cell((x + 2, y)).unwrap();
            if first.symbol() == "T" && second.symbol() == "a" && third.symbol() == "b" {
                highlighted_tab = first.fg == Color::Yellow;
            }
        }
    }

    assert!(highlighted_tab);
    fs::remove_file(path).unwrap();
}

#[test]
// include入口保持文本模式并按完整闭包校验保存。
fn include_entry_stays_text_mode_and_validates_full_closure() {
    let path = temporary_config();
    let fragment = path.with_file_name("procora-editor-fragment.yaml");
    fs::write(&fragment, "tasks:\n  worker:\n    command: worker\n").unwrap();
    fs::write(
        &path,
        "include: [procora-editor-fragment.yaml]\nversion: 1\nproject: demo\ntasks: {}\n",
    )
    .unwrap();
    let mut editor = ConfigEditor::open(&path).unwrap();

    assert!(editor.message().contains("多文件配置"));
    editor.handle_key(KeyEvent::new(KeyCode::F(1), KeyModifiers::NONE));
    assert!(editor.message().contains("避免表单展开"));
    editor.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));
    assert!(editor.message().starts_with("已保存"));
    assert!(fs::read_to_string(&path).unwrap().contains("include:"));

    fs::remove_file(path).unwrap();
    fs::remove_file(fragment).unwrap();
}

#[test]
// env_file在表单中保留声明路径且可直接切换文件，不会把文件内容写回内联env。
fn env_file_entry_is_editable_without_expanding_file_values() {
    let path = temporary_config();
    let env_file = path.with_extension("env");
    let next_env_file = path.with_extension("next.env");
    fs::write(&env_file, "SECRET=from-file\n").unwrap();
    fs::write(&next_env_file, "SECRET=from-next-file\nNEXT=yes\n").unwrap();
    fs::write(
        &path,
        format!(
            "version: 1\nproject: demo\ntasks:\n  api:\n    command: echo\n    env_file: {}\n",
            env_file.file_name().unwrap().to_string_lossy()
        ),
    )
    .unwrap();
    let mut editor = ConfigEditor::open(&path).unwrap();

    assert!(editor.message().contains("表单模式"));
    press(&mut editor, KeyCode::Tab);
    press(&mut editor, KeyCode::Enter);
    for _ in 0..4 {
        press(&mut editor, KeyCode::Tab);
    }
    backspace(
        &mut editor,
        env_file
            .file_name()
            .unwrap()
            .to_string_lossy()
            .chars()
            .count(),
    );
    type_text(
        &mut editor,
        &next_env_file.file_name().unwrap().to_string_lossy(),
    );
    press(&mut editor, KeyCode::Enter);
    editor.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));
    let saved = fs::read_to_string(&path).unwrap();
    let compiled = procora::config::load_path(&path).unwrap();
    let task = compiled.spec.tasks.values().next().unwrap();
    assert!(saved.contains("env_file:"));
    assert!(saved.contains(next_env_file.file_name().unwrap().to_str().unwrap()));
    assert!(!saved.contains("from-file"));
    assert!(!saved.contains("from-next-file"));
    assert_eq!(task.env["SECRET"], "from-next-file");
    assert_eq!(task.env["NEXT"], "yes");

    fs::remove_file(path).unwrap();
    fs::remove_file(env_file).unwrap();
    fs::remove_file(next_env_file).unwrap();
}

#[test]
// 打开有效配置后可通过表单新建并保存_task。
fn form_can_create_and_save_task_from_valid_config() {
    let path = temporary_config();
    fs::write(&path, "version: 1\nproject: demo\ntasks: {}\n").unwrap();
    let mut editor = ConfigEditor::open(&path).unwrap();

    editor.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    editor.handle_key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE));
    for character in "worker".chars() {
        editor.handle_key(KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE));
    }
    editor.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    for character in "echo".chars() {
        editor.handle_key(KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE));
    }
    for _ in 0..7 {
        editor.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    }
    editor.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    editor.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));

    let saved = fs::read_to_string(&path).unwrap();
    let compiled = procora::config::load_str(&saved, ConfigFormat::Yaml).unwrap();
    assert!(compiled.spec.tasks.contains_key(&"worker".parse().unwrap()));
    assert!(editor.message().starts_with("已保存"));
    fs::remove_file(path).unwrap();
}

#[test]
// 表单界面会显示任务和管理依赖弹窗入口。
fn form_shows_task_and_dependency_dialog_entries() {
    let path = temporary_config();
    fs::write(&path, "version: 1\nproject: demo\ntasks: {}\n").unwrap();
    let mut editor = ConfigEditor::open(&path).unwrap();
    editor.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    editor.handle_key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE));
    let backend = TestBackend::new(110, 30);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|frame| editor.render(frame)).unwrap();
    let text = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(Cell::symbol)
        .collect::<String>()
        .replace(' ', "");

    assert!(text.contains("结构化表单"));
    assert!(text.contains("新建Task"));
    assert!(text.contains("重启策略"));
    fs::remove_file(path).unwrap();
}

#[test]
// Task工作目录字段可在TUI中浏览子目录并保存为相对配置路径。
fn task_dialog_selects_working_directory() {
    let root = temporary_config().with_extension("");
    let work = root.join("work");
    let path = root.join("procora.yaml");
    fs::create_dir_all(&work).unwrap();
    fs::write(
        &path,
        "version: 1\nproject: demo\ntasks:\n  api:\n    command: api\n",
    )
    .unwrap();
    let mut editor = ConfigEditor::open(&path).unwrap();

    press(&mut editor, KeyCode::Tab);
    press(&mut editor, KeyCode::Enter);
    for _ in 0..3 {
        press(&mut editor, KeyCode::Down);
    }
    press(&mut editor, KeyCode::F(5));

    let backend = TestBackend::new(110, 30);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|frame| editor.render(frame)).unwrap();
    let text = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(Cell::symbol)
        .collect::<String>()
        .replace(' ', "");
    assert!(text.contains("选择运行目录"));
    assert!(text.contains("work/"));

    press(&mut editor, KeyCode::Down);
    press(&mut editor, KeyCode::Down);
    press(&mut editor, KeyCode::Enter);
    press(&mut editor, KeyCode::Enter);
    press(&mut editor, KeyCode::Enter);
    editor.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));

    let saved = fs::read_to_string(&path).unwrap();
    let compiled = procora::config::load_path(&path).unwrap();
    let task = compiled.spec.tasks.values().next().unwrap();
    let canonical_work = fs::canonicalize(&work).unwrap();
    assert_eq!(task.cwd.as_deref(), Some(canonical_work.as_path()));
    assert!(saved.contains("cwd: \"work\""));
    fs::remove_dir_all(root).unwrap();
}

#[test]
// 普通表单的单行文本字段会在当前输入末尾显示终端光标。
fn form_text_field_shows_cursor_at_input_end() {
    let path = temporary_config();
    fs::write(&path, "version: 1\nproject: demo\ntasks: {}\n").unwrap();
    let mut editor = ConfigEditor::open(&path).unwrap();
    press(&mut editor, KeyCode::Tab);
    press(&mut editor, KeyCode::Char('n'));
    let backend = TestBackend::new(110, 30);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|frame| editor.render(frame)).unwrap();
    let empty_cursor = terminal.backend().cursor_position();

    type_text(&mut editor, "api");
    terminal.draw(|frame| editor.render(frame)).unwrap();
    let input_cursor = terminal.backend().cursor_position();

    assert_eq!(input_cursor.y, empty_cursor.y);
    assert_eq!(input_cursor.x, empty_cursor.x + 3);
    assert!(input_cursor.x < 110 && input_cursor.y < 30);

    type_text(&mut editor, &"x".repeat(160));
    terminal.draw(|frame| editor.render(frame)).unwrap();
    let long_cursor = terminal.backend().cursor_position();
    let text = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(Cell::symbol)
        .collect::<String>();
    assert!(text.contains('…'));
    assert!(long_cursor.x < 109 && long_cursor.y < 30);
    fs::remove_file(path).unwrap();
}

#[test]
// 表单上下键在列表边界自动跨区，左右键不会改变当前区域。
fn form_navigation_uses_vertical_boundaries_and_keeps_horizontal_focus() {
    let path = temporary_config();
    fs::write(
        &path,
        "version: 1\nproject: demo\ntasks:\n  api:\n    command: api\n",
    )
    .unwrap();
    let mut editor = ConfigEditor::open(&path).unwrap();

    press(&mut editor, KeyCode::Down);
    press(&mut editor, KeyCode::Char('h'));
    press(&mut editor, KeyCode::Esc);
    press(&mut editor, KeyCode::Up);
    press(&mut editor, KeyCode::Char('h'));
    assert!(editor.message().contains("Task 区域"));

    press(&mut editor, KeyCode::Tab);
    press(&mut editor, KeyCode::Right);
    press(&mut editor, KeyCode::Char('n'));
    let backend = TestBackend::new(100, 28);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|frame| editor.render(frame)).unwrap();
    let text = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(Cell::symbol)
        .collect::<String>()
        .replace(' ', "");
    assert!(text.contains("新建Task"));
    fs::remove_file(path).unwrap();
}

#[test]
// 表单F3自动滚动会覆盖未聚焦区域内所有溢出的摘要文本。
fn form_auto_scroll_moves_global_overflowing_summaries() {
    let path = temporary_config();
    fs::write(
        &path,
        format!(
            "version: 1\nproject: demo\ntasks:\n  api:\n    command: prefix-{}\n",
            "x".repeat(100)
        ),
    )
    .unwrap();
    let mut editor = ConfigEditor::open(&path).unwrap();
    let backend = TestBackend::new(100, 28);
    let mut terminal = Terminal::new(backend).unwrap();

    terminal.draw(|frame| editor.render(frame)).unwrap();
    let initial = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .enumerate()
        .filter(|(index, _)| index % 100 < 42)
        .map(|(_, cell)| cell.symbol())
        .collect::<String>();
    assert!(initial.contains("prefix-"));

    press(&mut editor, KeyCode::F(3));
    assert!(editor.advance_auto_scroll(Duration::from_millis(2_500)));
    terminal.draw(|frame| editor.render(frame)).unwrap();
    let shifted = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .enumerate()
        .filter(|(index, _)| index % 100 < 42)
        .map(|(_, cell)| cell.symbol())
        .collect::<String>();
    assert!(!shifted.contains("prefix-"));
    fs::remove_file(path).unwrap();
}

#[test]
// 环境变量字段可用键值表编辑并保留包含分隔符的精确值。
fn task_environment_supports_key_value_table_editor() {
    let path = temporary_config();
    fs::write(
        &path,
        "version: 1\nproject: demo\ntasks:\n  api:\n    command: api\n",
    )
    .unwrap();
    let mut editor = ConfigEditor::open(&path).unwrap();

    press(&mut editor, KeyCode::Tab);
    press(&mut editor, KeyCode::Enter);
    for _ in 0..5 {
        press(&mut editor, KeyCode::Tab);
    }
    press(&mut editor, KeyCode::F(4));
    type_text(&mut editor, "TOKEN");
    press(&mut editor, KeyCode::Tab);
    type_text(&mut editor, "a=b,c");
    editor.handle_key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::CONTROL));
    type_text(&mut editor, "SECOND");
    press(&mut editor, KeyCode::Tab);
    type_text(&mut editor, "two");
    press(&mut editor, KeyCode::Enter);
    press(&mut editor, KeyCode::Enter);
    editor.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));

    let compiled = procora::config::load_path(&path).unwrap();
    assert_eq!(
        compiled.spec.tasks.values().next().unwrap().env["TOKEN"],
        "a=b,c"
    );
    assert_eq!(
        compiled.spec.tasks.values().next().unwrap().env["SECOND"],
        "two"
    );
    fs::remove_file(path).unwrap();
}

#[test]
// 普通字段的左右键移动真实光标，输入不再只能追加到末尾。
fn form_text_cursor_allows_mid_string_insertion() {
    let path = temporary_config();
    fs::write(
        &path,
        "version: 1\nproject: demo\ntasks:\n  api:\n    command: api\n",
    )
    .unwrap();
    let mut editor = ConfigEditor::open(&path).unwrap();

    press(&mut editor, KeyCode::Tab);
    press(&mut editor, KeyCode::Enter);
    press(&mut editor, KeyCode::Left);
    press(&mut editor, KeyCode::Char('x'));
    press(&mut editor, KeyCode::Enter);
    editor.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));

    let compiled = procora::config::load_path(&path).unwrap();
    assert!(compiled.spec.tasks.contains_key(&"apxi".parse().unwrap()));
    fs::remove_file(path).unwrap();
}

#[test]
// 表单保存支持_json_和_toml。
fn form_save_supports_json_and_toml() {
    for (extension, content, format) in [
        (
            "json",
            "{\"version\":1,\"project\":\"demo\",\"tasks\":{}}",
            ConfigFormat::Json,
        ),
        (
            "toml",
            "version = 1\nproject = \"demo\"\n[tasks]\n",
            ConfigFormat::Toml,
        ),
    ] {
        let path = temporary_config().with_extension(extension);
        fs::write(&path, content).unwrap();
        let mut editor = ConfigEditor::open(&path).unwrap();
        editor.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));
        assert!(procora::config::load_str(&fs::read_to_string(&path).unwrap(), format).is_ok());
        fs::remove_file(path).unwrap();
    }
}

#[test]
// 表单往返不会丢失健康检查和重启边界。
fn form_roundtrip_preserves_health_and_restart_limits() {
    let path = temporary_config();
    let initial = "version: 1\nproject: demo\ntasks:\n  api:\n    command: api\n    restart: on-failure\n    max_restarts: 7\n    restart_reset_after_ms: 1234\n    healthcheck:\n      command: checker\n      args: ['--ready']\n      period_ms: 25\n      timeout_ms: 10\n      success_threshold: 2\n      failure_threshold: 4\n";
    fs::write(&path, initial).unwrap();
    let mut editor = ConfigEditor::open(&path).unwrap();

    editor.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));

    let saved = fs::read_to_string(&path).unwrap();
    let compiled = procora::config::load_str(&saved, ConfigFormat::Yaml).unwrap();
    let task = compiled.spec.tasks.values().next().unwrap();
    let healthcheck = task.healthcheck.as_ref().unwrap();
    let procora::core::HealthCheckProbe::Exec { args, .. } = &healthcheck.probe else {
        panic!("应保留 exec 健康检查");
    };
    assert_eq!(args, &["--ready".to_owned()]);
    assert_eq!(healthcheck.success_threshold, 2);
    assert_eq!(healthcheck.failure_threshold, 4);
    assert_eq!(task.max_restarts, 7);
    assert_eq!(task.restart_reset_after_ms, 1234);
    fs::remove_file(path).unwrap();
}

#[test]
// 结构化编辑保存不会丢失HTTP健康检查字段。
fn form_roundtrip_preserves_http_health_check() {
    let path = temporary_config();
    fs::write(
        &path,
        "version: 1\nproject: demo\ntasks:\n  api:\n    command: api\n    healthcheck:\n      http_get:\n        scheme: http\n        host: localhost\n        port: 8080\n        path: /ready\n        headers:\n          X-Probe: yes\n        status_code: 204\n      period_ms: 250\n      timeout_ms: 100\n",
    )
    .unwrap();
    let mut editor = ConfigEditor::open(&path).unwrap();

    editor.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));

    let saved = fs::read_to_string(&path).unwrap();
    let compiled = procora::config::load_str(&saved, ConfigFormat::Yaml).unwrap();
    let check = compiled
        .spec
        .tasks
        .values()
        .next()
        .unwrap()
        .healthcheck
        .as_ref()
        .unwrap();
    let procora::core::HealthCheckProbe::HttpGet { http_get } = &check.probe else {
        panic!("应保留 HTTP GET 健康检查");
    };
    assert_eq!(http_get.host, "localhost");
    assert_eq!(http_get.port, Some(8080));
    assert_eq!(http_get.path, "/ready");
    assert_eq!(http_get.headers["X-Probe"], "yes");
    assert_eq!(http_get.status_code, 204);
    assert_eq!(check.period_ms, 250);
    assert_eq!(check.timeout_ms, 100);
    fs::remove_file(path).unwrap();
}

#[test]
// Task弹窗用JSON表示精确参数和环境值，编辑往返不会拆分空参数、空格或逗号。
fn task_dialog_preserves_precise_args_and_environment_values() {
    let path = temporary_config();
    fs::write(
        &path,
        "version: 1\nproject: demo\ntasks:\n  api:\n    command: api\n    args: ['hello world', '', 'quote\"value']\n    env:\n      CSV: 'a,b'\n      EQUAL: 'a=b'\n",
    )
    .unwrap();
    let mut editor = ConfigEditor::open(&path).unwrap();

    press(&mut editor, KeyCode::Tab);
    press(&mut editor, KeyCode::Enter);
    press(&mut editor, KeyCode::Enter);
    editor.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));

    let saved = fs::read_to_string(&path).unwrap();
    let compiled = procora::config::load_path(&path).unwrap();
    let task = compiled.spec.tasks.values().next().unwrap();
    assert_eq!(task.args, ["hello world", "", "quote\"value"]);
    assert_eq!(task.env["CSV"], "a,b");
    assert_eq!(task.env["EQUAL"], "a=b");
    assert!(saved.contains(r#"command: ["api","hello world","","quote\"value"]"#));
    assert!(!saved.contains("    args:"));
    fs::remove_file(path).unwrap();
}

#[test]
// Task弹窗可以直接编辑成功退出码并保持退出码0的固定成功语义。
fn task_dialog_edits_success_exit_codes() {
    let path = temporary_config();
    fs::write(
        &path,
        "version: 1\nproject: demo\ntasks:\n  api:\n    command: api\n",
    )
    .unwrap();
    let mut editor = ConfigEditor::open(&path).unwrap();

    press(&mut editor, KeyCode::Tab);
    press(&mut editor, KeyCode::Enter);
    for _ in 0..7 {
        press(&mut editor, KeyCode::Tab);
    }
    backspace(&mut editor, 3);
    type_text(&mut editor, "[130]");
    press(&mut editor, KeyCode::Enter);
    editor.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));

    let compiled = procora::config::load_path(&path).unwrap();
    let codes = &compiled
        .spec
        .tasks
        .values()
        .next()
        .unwrap()
        .success_exit_codes;
    assert_eq!(codes.iter().copied().collect::<Vec<_>>(), [0, 130]);
    fs::remove_file(path).unwrap();
}

#[test]
// h弹窗可以从无检查配置直接创建带精确请求头和状态码的HTTP检查。
fn health_dialog_creates_http_probe() {
    let path = temporary_config();
    fs::write(
        &path,
        "version: 1\nproject: demo\ntasks:\n  api:\n    command: api\n",
    )
    .unwrap();
    let mut editor = ConfigEditor::open(&path).unwrap();

    press(&mut editor, KeyCode::Tab);
    press(&mut editor, KeyCode::Char('h'));
    press(&mut editor, KeyCode::Right);
    press(&mut editor, KeyCode::Right);
    for _ in 0..3 {
        press(&mut editor, KeyCode::Tab);
    }
    type_text(&mut editor, "8080");
    press(&mut editor, KeyCode::Tab);
    press(&mut editor, KeyCode::Tab);
    backspace(&mut editor, 2);
    type_text(&mut editor, r#"{"X-Probe":"a,b"}"#);
    press(&mut editor, KeyCode::Tab);
    backspace(&mut editor, 3);
    type_text(&mut editor, "204");
    press(&mut editor, KeyCode::Enter);
    editor.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));

    let compiled = procora::config::load_path(&path).unwrap();
    let check = compiled
        .spec
        .tasks
        .values()
        .next()
        .unwrap()
        .healthcheck
        .as_ref()
        .unwrap();
    let procora::core::HealthCheckProbe::HttpGet { http_get } = &check.probe else {
        panic!("应创建 HTTP GET 健康检查");
    };
    assert_eq!(http_get.port, Some(8080));
    assert_eq!(http_get.headers["X-Probe"], "a,b");
    assert_eq!(http_get.status_code, 204);
    fs::remove_file(path).unwrap();
}

#[test]
// 健康检查弹窗只显示当前类型相关的Exec或HTTP字段。
fn health_dialog_hides_inactive_probe_fields() {
    let path = temporary_config();
    fs::write(
        &path,
        "version: 1\nproject: demo\ntasks:\n  api:\n    command: api\n",
    )
    .unwrap();
    let mut editor = ConfigEditor::open(&path).unwrap();
    press(&mut editor, KeyCode::Tab);
    press(&mut editor, KeyCode::Char('h'));
    let backend = TestBackend::new(100, 24);
    let mut terminal = Terminal::new(backend).unwrap();

    terminal.draw(|frame| editor.render(frame)).unwrap();
    let none = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(Cell::symbol)
        .collect::<String>()
        .replace(' ', "");
    assert!(!none.contains("Exec"));
    assert!(!none.contains("HTTP"));
    assert!(none.contains("1/6"));

    press(&mut editor, KeyCode::Right);
    terminal.draw(|frame| editor.render(frame)).unwrap();
    let exec = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(Cell::symbol)
        .collect::<String>()
        .replace(' ', "");
    assert!(exec.contains("Exec"));
    assert!(!exec.contains("HTTP"));
    assert!(exec.contains("1/9"));

    press(&mut editor, KeyCode::Right);
    terminal.draw(|frame| editor.render(frame)).unwrap();
    let http = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(Cell::symbol)
        .collect::<String>()
        .replace(' ', "");
    assert!(!http.contains("Exec"));
    assert!(http.contains("HTTP"));
    assert!(http.contains("1/12"));
    fs::remove_file(path).unwrap();
}

#[test]
// 小终端中的长健康检查弹窗会滚动到当前字段。
fn health_dialog_scrolls_selected_field_in_small_terminal() {
    let path = temporary_config();
    fs::write(
        &path,
        "version: 1\nproject: demo\ntasks:\n  api:\n    command: api\n",
    )
    .unwrap();
    let mut editor = ConfigEditor::open(&path).unwrap();
    press(&mut editor, KeyCode::Tab);
    press(&mut editor, KeyCode::Char('h'));
    press(&mut editor, KeyCode::Right);
    for _ in 0..8 {
        press(&mut editor, KeyCode::Down);
    }

    let backend = TestBackend::new(90, 14);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|frame| editor.render(frame)).unwrap();
    let text = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(Cell::symbol)
        .collect::<String>()
        .replace(' ', "");
    assert!(text.contains("连续失败阈值"));
    assert!(text.contains("9/9"));
    fs::remove_file(path).unwrap();
}

#[test]
// 表单删除确认可以用_esc_取消而不退出编辑器。
fn escape_cancels_form_delete_without_exiting() {
    let path = temporary_config();
    fs::write(
        &path,
        "version: 1\nproject: demo\ntasks:\n  worker:\n    command: echo\n",
    )
    .unwrap();
    let mut editor = ConfigEditor::open(&path).unwrap();
    editor.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    editor.handle_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE));
    editor.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));

    assert!(!editor.should_quit());
    assert!(editor.message().contains("取消删除"));
    fs::remove_file(path).unwrap();
}
