//! 引擎初始状态的公共行为测试。

use procora_config::{ConfigFormat, load_str};
use procora_engine::{DesiredState, Engine, ObservedState};

#[test]
fn 新引擎为每个任务建立等待运行状态() {
    let compiled = load_str(
        "version: 1\nproject: demo\ntasks:\n  api:\n    command: api\n",
        ConfigFormat::Yaml,
    )
    .unwrap();
    let task_id = compiled.spec.tasks.keys().next().unwrap().clone();
    let engine = Engine::new(&compiled.spec, compiled.graph);
    let state = engine.state(&task_id).unwrap();

    assert_eq!(state.desired, DesiredState::Running);
    assert_eq!(state.observed, ObservedState::Pending);
}
