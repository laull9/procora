//! 使用固定种子生成大量 DAG，验证调度和配置差异的性质。

use std::collections::{BTreeMap, BTreeSet};

use procora::{
    config::{ConfigFormat, diff_projects, load_str},
    engine::{Engine, EngineCommand, EngineEffect, ObservedState, RuntimeEvent},
};

/// 无外部依赖、跨平台稳定的伪随机生成器。
#[derive(Clone, Copy)]
struct Generator(u64);

impl Generator {
    /// 返回下一个确定性伪随机整数。
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.0
    }

    /// 返回小于上界的确定性索引。
    fn index(&mut self, upper: usize) -> usize {
        usize::try_from(self.next() % u64::try_from(upper).unwrap()).unwrap()
    }
}

#[test]
fn 随机无环图始终按依赖准入并按反向依赖停止() {
    let mut generator = Generator(0x0070_726f_636f_7261);
    for case in 0..192 {
        let task_count = 1 + generator.index(28);
        let yaml = generated_project(case, task_count, &mut generator);
        let compiled = load_str(&yaml, ConfigFormat::Yaml).unwrap();
        let spec = compiled.spec.clone();
        let mut engine = Engine::new(&compiled.spec, compiled.graph);
        let mut effects = engine.command(EngineCommand::StartAll);
        let mut spawned = BTreeSet::new();

        while !effects.is_empty() {
            let index = generator.index(effects.len());
            let effect = effects.swap_remove(index);
            let EngineEffect::Spawn {
                task_id, identity, ..
            } = effect
            else {
                panic!("启动阶段不应产生停止意图");
            };
            assert!(
                spec.tasks[&task_id]
                    .depends_on
                    .keys()
                    .all(|dependency| spawned.contains(dependency)),
                "case {case} 的 {task_id} 在依赖完成前被调度"
            );
            spawned.insert(task_id.clone());
            effects.extend(engine.event(RuntimeEvent::Spawned { task_id, identity }));
        }
        assert_eq!(spawned.len(), task_count);
        assert!(
            engine
                .states()
                .all(|(_, state)| state.observed == ObservedState::Running)
        );

        let stop_order = engine
            .command(EngineCommand::StopAll)
            .into_iter()
            .map(|effect| match effect {
                EngineEffect::Stop { task_id, .. } => task_id,
                EngineEffect::Spawn { .. } => panic!("停止阶段不应产生创建意图"),
            })
            .collect::<Vec<_>>();
        let positions = stop_order
            .iter()
            .enumerate()
            .map(|(index, task_id)| (task_id.clone(), index))
            .collect::<BTreeMap<_, _>>();
        for (task_id, task) in &spec.tasks {
            for dependency in task.depends_on.keys() {
                assert!(
                    positions[task_id] < positions[dependency],
                    "case {case} 必须先停止下游 {task_id} 再停止 {dependency}"
                );
            }
        }

        let diff = diff_projects(&spec, &spec);
        assert!(diff.is_empty());
        assert_eq!(diff.unchanged.len(), task_count);
    }
}

/// 生成只包含指向更早 Task 依赖边的合法 DAG。
fn generated_project(case: usize, task_count: usize, generator: &mut Generator) -> String {
    let mut yaml = format!("version: 1\nproject: property-{case}\ntasks:\n");
    for index in 0..task_count {
        use std::fmt::Write as _;
        writeln!(yaml, "  task-{index}:").unwrap();
        writeln!(yaml, "    command: executable").unwrap();
        let dependencies = (0..index)
            .filter(|_| generator.next().is_multiple_of(5))
            .collect::<Vec<_>>();
        if !dependencies.is_empty() {
            writeln!(yaml, "    depends_on:").unwrap();
            for dependency in dependencies {
                writeln!(yaml, "      task-{dependency}: {{}}").unwrap();
            }
        }
    }
    yaml
}
