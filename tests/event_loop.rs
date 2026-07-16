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
