//! 配置语义差异分类与下游影响传播测试。

use procora::config::{ConfigFormat, diff_projects, load_str};

/// 编译便于比较的 YAML 项目配置。
fn compile(tasks: &str) -> procora::config::CompiledProject {
    load_str(
        &format!("version: 1\nproject: demo\ntasks:\n{tasks}"),
        ConfigFormat::Yaml,
    )
    .unwrap()
}

#[test]
// 进程身份变化会传播到全部下游但不波及无关任务。
fn process_identity_changes_propagate_only_to_downstream_tasks() {
    let current = compile(
        "  database:\n    command: postgres\n  api:\n    command: api\n    depends_on:\n      database: {}\n  docs:\n    command: docs\n",
    );
    let candidate = compile(
        "  database:\n    command: postgres-next\n  api:\n    command: api\n    depends_on:\n      database: {}\n  docs:\n    command: docs\n",
    );

    let diff = diff_projects(&current.spec, &candidate.spec);

    assert_eq!(
        diff.restart
            .iter()
            .map(procora::core::TaskId::as_str)
            .collect::<Vec<_>>(),
        ["api", "database"]
    );
    assert_eq!(
        diff.unchanged
            .iter()
            .map(procora::core::TaskId::as_str)
            .collect::<Vec<_>>(),
        ["docs"]
    );
}

#[test]
// 纯运行策略变化被分类为原地更新。
fn runtime_policy_changes_are_in_place_updates() {
    let current = compile("  worker:\n    command: worker\n");
    let candidate = compile(
        "  worker:\n    command: worker\n    restart: always\n    restart_delay_ms: 1200\n    max_restarts: 5\n    restart_reset_after_ms: 30000\n",
    );

    let diff = diff_projects(&current.spec, &candidate.spec);

    assert!(diff.restart.is_empty());
    assert_eq!(diff.update_in_place[0].as_str(), "worker");
    assert!(diff.unchanged.is_empty());
}

#[test]
// 新增删除和无变化集合保持稳定排序。
fn diff_task_sets_keep_stable_ordering() {
    let current = compile("  alpha:\n    command: a\n  old:\n    command: old\n");
    let candidate = compile("  alpha:\n    command: a\n  new:\n    command: new\n");

    let diff = diff_projects(&current.spec, &candidate.spec);

    assert_eq!(diff.added[0].as_str(), "new");
    assert_eq!(diff.removed[0].as_str(), "old");
    assert_eq!(diff.unchanged[0].as_str(), "alpha");
}
