//! 多种配置前端的等价性测试。

use procora::config::{ConfigFormat, load_str};

const YAML: &str = r#"
version: 1
project: demo
tasks:
  database:
    command: postgres
  api:
    command: api
    args: ["--port", "8080"]
    depends_on:
      database:
        condition: started
"#;

const TOML: &str = r#"
version = 1
project = "demo"

[tasks.database]
command = "postgres"

[tasks.api]
command = "api"
args = ["--port", "8080"]

[tasks.api.depends_on.database]
condition = "started"
"#;

const JSON: &str = r#"
{
  "version": 1,
  "project": "demo",
  "tasks": {
    "database": { "command": "postgres" },
    "api": {
      "command": "api",
      "args": ["--port", "8080"],
      "depends_on": { "database": { "condition": "started" } }
    }
  }
}
"#;

#[test]
fn 三种声明式格式产生相同规范() {
    let yaml = load_str(YAML, ConfigFormat::Yaml).unwrap();
    let toml = load_str(TOML, ConfigFormat::Toml).unwrap();
    let json = load_str(JSON, ConfigFormat::Json).unwrap();

    assert_eq!(yaml.spec, toml.spec);
    assert_eq!(toml.spec, json.spec);
    assert_eq!(yaml.graph.start_order(), toml.graph.start_order());
}

#[test]
fn 未知字段会被拒绝() {
    let invalid = "version: 1\nproject: demo\ntasks: {}\nunexpected: true\n";

    assert!(load_str(invalid, ConfigFormat::Yaml).is_err());
}
