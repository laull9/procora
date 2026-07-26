//! 单写者事件身份、依赖条件与重启策略测试。

use procora::config::{ConfigFormat, load_str};
use procora::engine::{Engine, EngineCommand, EngineEffect, HealthState, RuntimeEvent};

/// 从单个 Spawn 意图中提取 Task 和身份。
fn spawn(effect: &EngineEffect) -> (&procora::core::TaskId, procora::engine::TaskRunIdentity) {
    let EngineEffect::Spawn {
        task_id, identity, ..
    } = effect
    else {
        panic!("应生成启动意图");
    };
    (task_id, *identity)
}

#[test]
// started依赖在上游创建后立即调度。
fn started_dependency_schedules_after_upstream_spawn() {
    let compiled = load_str(
        "version: 1\nproject: demo\ntasks:\n  first:\n    command: echo\n  second:\n    command: echo\n    depends_on:\n      first:\n        condition: started\n",
        ConfigFormat::Yaml,
    )
    .unwrap();
    let mut engine = Engine::new(&compiled.spec, compiled.graph);
    let first = engine.command(EngineCommand::StartAll);
    assert_eq!(first.len(), 1);
    let (task_id, identity) = spawn(&first[0]);
    assert_eq!(task_id.as_str(), "first");

    let next = engine.event(RuntimeEvent::Spawned {
        task_id: task_id.clone(),
        identity,
    });
    assert_eq!(spawn(&next[0]).0.as_str(), "second");
}

#[test]
// completed依赖等待上游成功退出。
fn completed_dependency_waits_for_successful_exit() {
    let compiled = load_str(
        "version: 1\nproject: demo\ntasks:\n  prepare:\n    command: echo\n  app:\n    command: echo\n    depends_on:\n      prepare:\n        condition: completed_successfully\n",
        ConfigFormat::Yaml,
    )
    .unwrap();
    let mut engine = Engine::new(&compiled.spec, compiled.graph);
    let first = engine.command(EngineCommand::StartAll);
    let (task_id, identity) = spawn(&first[0]);
    assert!(
        engine
            .event(RuntimeEvent::Spawned {
                task_id: task_id.clone(),
                identity,
            })
            .is_empty()
    );
    let next = engine.event(RuntimeEvent::Exited {
        task_id: task_id.clone(),
        identity,
        exit_code: Some(0),
        success: true,
        run_duration_ms: 1,
    });
    assert_eq!(spawn(&next[0]).0.as_str(), "app");
}

#[test]
// healthy依赖等待匹配run的健康事件。
fn healthy_dependency_waits_for_matching_run() {
    let compiled = load_str(
        "version: 1\nproject: demo\ntasks:\n  first:\n    command: echo\n  second:\n    command: echo\n    depends_on:\n      first:\n        condition: healthy\n",
        ConfigFormat::Yaml,
    )
    .unwrap();
    let mut engine = Engine::new(&compiled.spec, compiled.graph);
    let first = engine.command(EngineCommand::StartAll);
    let (task_id, identity) = spawn(&first[0]);
    assert!(
        engine
            .event(RuntimeEvent::HealthChanged {
                task_id: task_id.clone(),
                identity,
                health: HealthState::Starting,
                detail: None,
            })
            .is_empty()
    );
    assert!(
        engine
            .event(RuntimeEvent::Spawned {
                task_id: task_id.clone(),
                identity,
            })
            .is_empty()
    );

    let next = engine.event(RuntimeEvent::HealthChanged {
        task_id: task_id.clone(),
        identity,
        health: HealthState::Healthy,
        detail: None,
    });
    assert_eq!(spawn(&next[0]).0.as_str(), "second");
}

#[test]
// 迟到事件不能覆盖新generation。
fn late_events_cannot_override_new_generation() {
    let compiled = load_str(
        "version: 1\nproject: demo\ntasks:\n  app:\n    command: echo\n",
        ConfigFormat::Yaml,
    )
    .unwrap();
    let mut engine = Engine::new(&compiled.spec, compiled.graph);
    let old = engine.command(EngineCommand::StartAll);
    let (task_id, old_identity) = spawn(&old[0]);
    let new = engine.command(EngineCommand::StartAll);
    let (_, new_identity) = spawn(&new[0]);

    assert!(
        engine
            .event(RuntimeEvent::Exited {
                task_id: task_id.clone(),
                identity: old_identity,
                exit_code: Some(1),
                success: false,
                run_duration_ms: 1,
            })
            .is_empty()
    );
    assert_eq!(
        engine.state(task_id).unwrap().run_id,
        Some(new_identity.run_id)
    );
}

#[test]
// on_failure使用有界退避重新调度。
fn on_failure_uses_bounded_retry_backoff() {
    let compiled = load_str(
        "version: 1\nproject: demo\ntasks:\n  app:\n    command: missing\n    restart: on-failure\n    restart_delay_ms: 25\n",
        ConfigFormat::Yaml,
    )
    .unwrap();
    let mut engine = Engine::new(&compiled.spec, compiled.graph);
    let first = engine.command(EngineCommand::StartAll);
    let (task_id, identity) = spawn(&first[0]);
    let retry = engine.event(RuntimeEvent::SpawnFailed {
        task_id: task_id.clone(),
        identity,
    });
    let EngineEffect::Spawn { delay_ms, .. } = retry[0] else {
        panic!("应产生重启意图");
    };
    assert_eq!(delay_ms, 25);
}

#[test]
// 自动重启达到上限后停止，放宽上限可原地恢复，手动启动会清零计数。
fn restart_limit_stops_resumes_and_resets() {
    let compiled = load_str(
        "version: 1\nproject: demo\ntasks:\n  app:\n    command: missing\n    restart: on-failure\n    restart_delay_ms: 25\n    max_restarts: 2\n    restart_reset_after_ms: 0\n",
        ConfigFormat::Yaml,
    )
    .unwrap();
    let mut engine = Engine::new(&compiled.spec, compiled.graph);
    let first = engine.command(EngineCommand::StartAll);
    let (task_id, first_identity) = spawn(&first[0]);

    let second = engine.event(RuntimeEvent::SpawnFailed {
        task_id: task_id.clone(),
        identity: first_identity,
    });
    let (task_id, second_identity) = spawn(&second[0]);
    assert!(matches!(
        second[0],
        EngineEffect::Spawn { delay_ms: 25, .. }
    ));
    let third = engine.event(RuntimeEvent::SpawnFailed {
        task_id: task_id.clone(),
        identity: second_identity,
    });
    let (task_id, third_identity) = spawn(&third[0]);
    assert!(matches!(third[0], EngineEffect::Spawn { delay_ms: 50, .. }));
    assert!(
        engine
            .event(RuntimeEvent::SpawnFailed {
                task_id: task_id.clone(),
                identity: third_identity,
            })
            .is_empty()
    );
    let exhausted = engine.state(task_id).unwrap();
    assert_eq!(exhausted.restart_attempt, 2);
    assert!(exhausted.restart_exhausted);

    let relaxed = load_str(
        "version: 1\nproject: demo\ntasks:\n  app:\n    command: missing\n    restart: on-failure\n    restart_delay_ms: 25\n    max_restarts: 3\n    restart_reset_after_ms: 0\n",
        ConfigFormat::Yaml,
    )
    .unwrap();
    let resumed = engine.update_runtime_policies(&relaxed.spec);
    assert!(matches!(
        resumed[0],
        EngineEffect::Spawn { delay_ms: 100, .. }
    ));
    assert!(!engine.state(task_id).unwrap().restart_exhausted);

    let manual = engine.command(EngineCommand::StartAll);
    assert!(matches!(manual[0], EngineEffect::Spawn { delay_ms: 0, .. }));
    assert_eq!(engine.state(task_id).unwrap().restart_attempt, 0);
}

#[test]
// 单次运行达到稳定窗口后从首次退避重新计算连续重启。
fn stable_run_resets_consecutive_restart_count() {
    let compiled = load_str(
        "version: 1\nproject: demo\ntasks:\n  app:\n    command: app\n    restart: on-failure\n    restart_delay_ms: 25\n    max_restarts: 1\n    restart_reset_after_ms: 1000\n",
        ConfigFormat::Yaml,
    )
    .unwrap();
    let mut engine = Engine::new(&compiled.spec, compiled.graph);
    let first = engine.command(EngineCommand::StartAll);
    let (task_id, first_identity) = spawn(&first[0]);
    let retry = engine.event(RuntimeEvent::SpawnFailed {
        task_id: task_id.clone(),
        identity: first_identity,
    });
    let (task_id, retry_identity) = spawn(&retry[0]);

    let after_stable_run = engine.event(RuntimeEvent::Exited {
        task_id: task_id.clone(),
        identity: retry_identity,
        exit_code: Some(1),
        success: false,
        run_duration_ms: 1000,
    });

    assert!(matches!(
        after_stable_run[0],
        EngineEffect::Spawn { delay_ms: 25, .. }
    ));
    let state = engine.state(task_id).unwrap();
    assert_eq!(state.restart_attempt, 1);
    assert!(!state.restart_exhausted);
}
