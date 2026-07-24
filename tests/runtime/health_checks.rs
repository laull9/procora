//! 健康检查配置、阈值和真实依赖门控测试。

use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use procora::{
    config::{ConfigFormat, load_str},
    core::HealthCheckProbe,
    daemon::ServiceHost,
    protocol::{SnapshotSourceDto, TaskHealthDto},
};
use serde_json::json;

#[path = "../support/mod.rs"]
mod support;
use support::http::{HttpFixture, HttpMode};

/// 同进程并行测试的临时目录序号。
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// 创建健康检查测试独占目录。
fn temporary_directory() -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("系统时间应晚于 Unix 纪元")
        .as_nanos();
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let directory = std::env::temp_dir().join(format!(
        "procora-health-{}-{nonce}-{sequence}",
        std::process::id()
    ));
    fs::create_dir_all(&directory).expect("应能创建测试目录");
    directory
}

/// 构造由当前测试二进制承担 Task 与检查程序的跨平台配置。
fn runtime_configuration(directory: &Path) -> String {
    let executable = std::env::current_exe().expect("应能读取当前测试二进制路径");
    let ready = directory.join("ready");
    let dependent = directory.join("dependent");
    json!({
        "version": 1,
        "project": "health-runtime",
        "tasks": {
            "server": {
                "command": executable,
                "args": ["--exact", "health_checks::long_running_task_helper", "--nocapture"],
                "env": {
                    "PROCORA_HEALTH_TEST": "1",
                    "PROCORA_READY_FILE": ready,
                },
                "shutdown_timeout_ms": 500,
                "healthcheck": {
                    "command": executable,
                    "args": ["--exact", "health_checks::health_check_helper", "--nocapture"],
                    "period_ms": 20,
                    "timeout_ms": 500,
                    "success_threshold": 2,
                    "failure_threshold": 2,
                }
            },
            "dependent": {
                "command": executable,
                "args": ["--exact", "health_checks::dependent_task_helper", "--nocapture"],
                "env": {
                    "PROCORA_HEALTH_TEST": "1",
                    "PROCORA_DEPENDENT_FILE": dependent,
                },
                "depends_on": {
                    "server": { "condition": "healthy" }
                }
            }
        }
    })
    .to_string()
}

#[test]
// 健康检查配置应用默认值并保留参数数组。
fn health_check_defaults_preserve_argument_array() {
    let compiled = load_str(
        "version: 1\nproject: demo\ntasks:\n  api:\n    command: api\n    healthcheck:\n      command: checker\n      args: ['--ready']\n",
        ConfigFormat::Yaml,
    )
    .expect("有效检查应通过配置编译");
    let healthcheck = compiled
        .spec
        .tasks
        .values()
        .next()
        .unwrap()
        .healthcheck
        .as_ref()
        .unwrap();

    let HealthCheckProbe::Exec { command, args, cwd } = &healthcheck.probe else {
        panic!("应编译为 exec 探针");
    };
    assert_eq!(command, "checker");
    assert_eq!(args, &["--ready".to_owned()]);
    assert!(cwd.is_none());
    assert_eq!(healthcheck.period_ms, 10_000);
    assert_eq!(healthcheck.timeout_ms, 1_000);
    assert_eq!(healthcheck.success_threshold, 1);
    assert_eq!(healthcheck.failure_threshold, 3);
}

#[test]
// 健康检查拒绝无界时间和阈值。
fn health_checks_reject_unbounded_limits() {
    let error = load_str(
        "version: 1\nproject: demo\ntasks:\n  api:\n    command: api\n    healthcheck:\n      command: checker\n      period_ms: 0\n      timeout_ms: 300001\n      success_threshold: 0\n      failure_threshold: 101\n",
        ConfigFormat::Yaml,
    )
    .expect_err("无界检查配置必须被拒绝")
    .to_string();

    for field in [
        "healthcheck.period_ms",
        "healthcheck.timeout_ms",
        "healthcheck.success_threshold",
        "healthcheck.failure_threshold",
    ] {
        assert!(error.contains(field), "缺少字段诊断：{field}: {error}");
    }
}

#[test]
// HTTP检查应用确定默认值并拒绝混合探针与无效请求字段。
fn http_health_check_defaults_and_validation_are_precise() {
    let compiled = load_str(
        "version: 1\nproject: demo\ntasks:\n  api:\n    command: api\n    healthcheck:\n      http_get:\n        port: 8080\n",
        ConfigFormat::Yaml,
    )
    .expect("有效 HTTP 检查应通过配置编译");
    let check = compiled
        .spec
        .tasks
        .values()
        .next()
        .unwrap()
        .healthcheck
        .as_ref()
        .unwrap();
    let HealthCheckProbe::HttpGet { http_get } = &check.probe else {
        panic!("应编译为 HTTP GET 探针");
    };
    assert_eq!(http_get.scheme.as_str(), "http");
    assert_eq!(http_get.host, "127.0.0.1");
    assert_eq!(http_get.port, Some(8080));
    assert_eq!(http_get.path, "/");
    assert_eq!(http_get.status_code, 200);
    let effective = serde_json::to_value(&compiled.spec).expect("规范化配置应可序列化");
    assert_eq!(
        effective["tasks"]["api"]["healthcheck"]["http_get"]["host"],
        "127.0.0.1"
    );
    assert!(
        effective["tasks"]["api"]["healthcheck"]
            .get("command")
            .is_none()
    );

    let invalid = json!({
        "version": 1,
        "project": "demo",
        "tasks": {
            "api": {
                "command": "api",
                "healthcheck": {
                    "command": "checker",
                    "http_get": {
                        "host": "http://bad-host",
                        "port": 0,
                        "path": "relative path",
                        "status_code": 404,
                        "headers": { "Bad Name": "bad\nvalue" }
                    }
                }
            }
        }
    })
    .to_string();
    let error = load_str(&invalid, ConfigFormat::Json)
        .expect_err("混合或无效 HTTP 检查必须被拒绝")
        .to_string();
    for message in [
        "command 与 http_get",
        "healthcheck.http_get.host",
        "healthcheck.http_get.port",
        "healthcheck.http_get.path",
        "healthcheck.http_get.status_code",
        "healthcheck.http_get.headers.Bad Name",
    ] {
        assert!(error.contains(message), "缺少诊断 {message}: {error}");
    }
}

#[test]
// YAML、TOML和JSON中的HTTP检查会编译成相同领域配置。
fn http_health_check_is_equivalent_across_formats() {
    let inputs = [
        (
            ConfigFormat::Yaml,
            "version: 1\nproject: demo\ntasks:\n  api:\n    command: api\n    healthcheck:\n      http_get:\n        scheme: https\n        host: localhost\n        port: 8443\n        path: /ready?full=1\n        headers:\n          X-Probe: yes\n        status_code: 204\n",
        ),
        (
            ConfigFormat::Toml,
            "version = 1\nproject = \"demo\"\n[tasks.api]\ncommand = \"api\"\n[tasks.api.healthcheck.http_get]\nscheme = \"https\"\nhost = \"localhost\"\nport = 8443\npath = \"/ready?full=1\"\nstatus_code = 204\n[tasks.api.healthcheck.http_get.headers]\nX-Probe = \"yes\"\n",
        ),
        (
            ConfigFormat::Json,
            r#"{"version":1,"project":"demo","tasks":{"api":{"command":"api","healthcheck":{"http_get":{"scheme":"https","host":"localhost","port":8443,"path":"/ready?full=1","headers":{"X-Probe":"yes"},"status_code":204}}}}}"#,
        ),
    ];
    let probes = inputs.map(|(format, input)| {
        load_str(input, format)
            .expect("各格式 HTTP 检查都应有效")
            .spec
            .tasks
            .into_values()
            .next()
            .unwrap()
            .healthcheck
            .unwrap()
    });
    assert_eq!(probes[0], probes[1]);
    assert_eq!(probes[1], probes[2]);
}

#[test]
// 连续健康后才启动依赖任务。
fn dependent_task_starts_after_consecutive_health_successes() {
    let directory = temporary_directory();
    let compiled =
        load_str(&runtime_configuration(&directory), ConfigFormat::Json).expect("运行配置应有效");
    let mut host = ServiceHost::from_compiled_at(compiled, &directory);
    host.start().expect("服务应能启动");

    let dependent = directory.join("dependent");
    let deadline = Instant::now() + Duration::from_secs(5);
    let snapshot = loop {
        let snapshot = host.snapshot(SnapshotSourceDto::CenterLive, true);
        if dependent.exists() {
            break snapshot;
        }
        assert!(Instant::now() < deadline, "健康依赖没有在期限内放行");
        thread::sleep(Duration::from_millis(10));
    };
    let server = snapshot
        .tasks
        .iter()
        .find(|task| task.task_id.as_str() == "server")
        .expect("应包含 server Task");
    assert_eq!(server.health, TaskHealthDto::Healthy);

    host.stop().expect("服务应能停止并取消检查");
    fs::remove_dir_all(directory).expect("应能清理测试目录");
}

#[test]
// 未声明cwd的exec健康检查继承包含非ASCII字符的service根目录。
fn exec_health_check_without_cwd_uses_unicode_service_root() {
    let parent = temporary_directory();
    let directory = parent.join("下载 目录");
    fs::create_dir_all(&directory).expect("应能创建非 ASCII 服务目录");
    let executable = std::env::current_exe().expect("应能读取当前测试二进制路径");
    let configuration = json!({
        "version": 1,
        "project": "health-cwd",
        "tasks": {
            "server": {
                "command": executable,
                "args": ["--exact", "health_checks::long_running_task_helper", "--nocapture"],
                "env": {
                    "PROCORA_HEALTH_TEST": "1",
                    "PROCORA_READY_FILE": directory.join("ready"),
                    "PROCORA_EXPECTED_CWD": directory,
                },
                "shutdown_timeout_ms": 500,
                "healthcheck": {
                    "command": executable,
                    "args": [
                        "--exact",
                        "health_checks::health_check_cwd_helper",
                        "--nocapture"
                    ],
                    "period_ms": 20,
                    "timeout_ms": 500
                }
            }
        }
    })
    .to_string();
    let compiled = load_str(&configuration, ConfigFormat::Json).expect("运行配置应有效");
    let mut host = ServiceHost::from_compiled_at(compiled, &directory);
    host.start().expect("服务应能启动");

    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let snapshot = host.snapshot(SnapshotSourceDto::CenterLive, true);
        if snapshot.tasks[0].health == TaskHealthDto::Healthy {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "exec 健康检查没有在 service 根目录成功执行"
        );
        thread::sleep(Duration::from_millis(10));
    }

    host.stop().expect("服务应能停止并取消检查");
    fs::remove_dir_all(parent).expect("应能清理测试目录");
}

#[test]
// HTTP连续就绪结果会放行healthy依赖并携带声明的请求头。
fn http_readiness_releases_dependent_after_threshold() {
    let directory = temporary_directory();
    let ready = directory.join("ready");
    let dependent = directory.join("dependent");
    let server = HttpFixture::start(HttpMode::ReadyWhen(ready.clone()));
    let executable = std::env::current_exe().expect("应能读取当前测试二进制路径");
    let configuration = json!({
        "version": 1,
        "project": "http-health-runtime",
        "tasks": {
            "server": {
                "command": executable,
                "args": ["--exact", "health_checks::long_running_task_helper", "--nocapture"],
                "env": {
                    "PROCORA_HEALTH_TEST": "1",
                    "PROCORA_READY_FILE": ready,
                },
                "shutdown_timeout_ms": 500,
                "healthcheck": {
                    "http_get": {
                        "port": server.port(),
                        "path": "/ready",
                        "headers": { "X-Probe": "yes" },
                        "status_code": 204
                    },
                    "period_ms": 20,
                    "timeout_ms": 500,
                    "success_threshold": 2,
                    "failure_threshold": 2
                }
            },
            "dependent": {
                "command": executable,
                "args": ["--exact", "health_checks::dependent_task_helper", "--nocapture"],
                "env": {
                    "PROCORA_HEALTH_TEST": "1",
                    "PROCORA_DEPENDENT_FILE": dependent,
                },
                "depends_on": { "server": { "condition": "healthy" } }
            }
        }
    })
    .to_string();
    let compiled = load_str(&configuration, ConfigFormat::Json).expect("HTTP 运行配置应有效");
    let mut host = ServiceHost::from_compiled_at(compiled, &directory);
    host.start().expect("服务应能启动");

    let deadline = Instant::now() + Duration::from_secs(5);
    let snapshot = loop {
        let snapshot = host.snapshot(SnapshotSourceDto::CenterLive, true);
        if dependent.exists() {
            break snapshot;
        }
        assert!(Instant::now() < deadline, "HTTP 就绪依赖没有在期限内放行");
        thread::sleep(Duration::from_millis(10));
    };
    let task = snapshot
        .tasks
        .iter()
        .find(|task| task.task_id.as_str() == "server")
        .expect("应包含 server Task");
    assert_eq!(task.health, TaskHealthDto::Healthy);
    host.stop().expect("服务应能停止");
    drop(server);
    fs::remove_dir_all(directory).expect("应能清理测试目录");
}

#[test]
// 停止Task不会等待阻塞中的HTTP检查走完整个请求超时。
fn stopping_task_cancels_http_probe_without_blocking() {
    let directory = temporary_directory();
    let server = HttpFixture::start(HttpMode::Hang);
    let executable = std::env::current_exe().expect("应能读取当前测试二进制路径");
    let configuration = json!({
        "version": 1,
        "project": "http-health-cancel",
        "tasks": {
            "server": {
                "command": executable,
                "args": ["--exact", "health_checks::long_running_task_helper", "--nocapture"],
                "env": {
                    "PROCORA_HEALTH_TEST": "1",
                    "PROCORA_READY_FILE": directory.join("ready"),
                },
                "shutdown_timeout_ms": 500,
                "healthcheck": {
                    "http_get": { "port": server.port() },
                    "period_ms": 100,
                    "timeout_ms": 5000
                }
            }
        }
    })
    .to_string();
    let compiled = load_str(&configuration, ConfigFormat::Json).expect("HTTP 运行配置应有效");
    let mut host = ServiceHost::from_compiled_at(compiled, &directory);
    host.start().expect("服务应能启动");
    let deadline = Instant::now() + Duration::from_secs(2);
    while !server.accepted() {
        host.refresh();
        assert!(Instant::now() < deadline, "HTTP 检查没有建立连接");
        thread::sleep(Duration::from_millis(10));
    }

    let started = Instant::now();
    host.stop().expect("停止不应等待 HTTP 请求超时");
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "停止被 HTTP 检查阻塞"
    );
    drop(server);
    fs::remove_dir_all(directory).expect("应能清理测试目录");
}

/// 被真实 Task 子进程调用：延迟创建就绪文件并保持运行。
#[test]
// 长期任务辅助进程。
fn long_running_task_helper() {
    if std::env::var_os("PROCORA_HEALTH_TEST").is_none() {
        return;
    }
    thread::sleep(Duration::from_millis(120));
    fs::write(
        std::env::var_os("PROCORA_READY_FILE").expect("应传入就绪文件"),
        b"ready",
    )
    .expect("应能写入就绪文件");
    thread::sleep(Duration::from_secs(2));
}

/// 被健康检查子进程调用：以文件是否出现决定退出状态。
#[test]
// 健康检查辅助进程。
fn health_check_helper() {
    if std::env::var_os("PROCORA_HEALTH_TEST").is_none() {
        return;
    }
    let ready = std::env::var_os("PROCORA_READY_FILE").expect("应传入就绪文件");
    assert!(Path::new(&ready).exists(), "服务尚未就绪");
}

/// 被健康检查子进程调用：验证未显式配置时继承服务工作目录。
#[test]
// 健康检查工作目录辅助进程。
fn health_check_cwd_helper() {
    if std::env::var_os("PROCORA_HEALTH_TEST").is_none() {
        return;
    }
    let expected =
        PathBuf::from(std::env::var_os("PROCORA_EXPECTED_CWD").expect("应传入预期工作目录"));
    assert_eq!(
        fs::canonicalize(std::env::current_dir().expect("应能读取当前工作目录"))
            .expect("当前工作目录应可规范化"),
        fs::canonicalize(expected).expect("预期工作目录应可规范化")
    );
}

/// 被下游 Task 子进程调用：记录依赖已经放行。
#[test]
// 依赖任务辅助进程。
fn dependent_task_helper() {
    if std::env::var_os("PROCORA_HEALTH_TEST").is_none() {
        return;
    }
    fs::write(
        std::env::var_os("PROCORA_DEPENDENT_FILE").expect("应传入下游标记文件"),
        b"started",
    )
    .expect("应能写入下游标记文件");
}
