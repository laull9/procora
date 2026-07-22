//! 任务依赖图的公共行为测试。

use std::{
    collections::{BTreeMap, BTreeSet},
    str::FromStr,
};

use procora::core::{
    DependencySpec, GraphError, ProjectSpec, RestartPolicy, TaskGraph, TaskId, TaskSpec,
};

/// 创建测试任务规范。
fn task(dependencies: &[&str]) -> TaskSpec {
    TaskSpec {
        command: "echo".to_owned(),
        args: Vec::new(),
        cwd: None,
        env: BTreeMap::new(),
        healthcheck: None,
        success_exit_codes: BTreeSet::from([0]),
        depends_on: dependencies
            .iter()
            .map(|id| (TaskId::from_str(id).unwrap(), DependencySpec::default()))
            .collect(),
        restart: RestartPolicy::Never,
        restart_delay_ms: 500,
        max_restarts: 0,
        restart_reset_after_ms: 60_000,
        shutdown_timeout_ms: 5_000,
    }
}

#[test]
// 按依赖顺序启动并反向停止。
fn tasks_start_in_dependency_order_and_stop_in_reverse() {
    let database = TaskId::from_str("database").unwrap();
    let api = TaskId::from_str("api").unwrap();
    let spec = ProjectSpec {
        version: 1,
        project: "demo".to_owned(),
        tasks: BTreeMap::from([
            (database.clone(), task(&[])),
            (api.clone(), task(&["database"])),
        ]),
    };

    let graph = TaskGraph::compile(&spec).unwrap();

    assert_eq!(graph.start_order(), &[database.clone(), api.clone()]);
    assert_eq!(
        graph.stop_order().cloned().collect::<Vec<_>>(),
        vec![api, database]
    );
}

#[test]
// 拒绝循环依赖。
fn cyclic_dependencies_are_rejected() {
    let spec = ProjectSpec {
        version: 1,
        project: "cycle".to_owned(),
        tasks: BTreeMap::from([
            (TaskId::from_str("a").unwrap(), task(&["b"])),
            (TaskId::from_str("b").unwrap(), task(&["a"])),
        ]),
    };

    assert!(matches!(
        TaskGraph::compile(&spec),
        Err(GraphError::Cycle { .. })
    ));
}
