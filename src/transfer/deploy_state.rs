//! 全托管 Service 的 release 状态与部署记录。

use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, bail};
use serde::{Deserialize, Serialize};

/// 单个已接收 release 的持久记录。
#[derive(Clone, Debug, Deserialize, Serialize)]
pub(super) struct ReleaseRecord {
    pub(super) id: String,
    pub(super) sha256: String,
    #[serde(default)]
    pub(super) config_path: PathBuf,
    pub(super) deployed_at_ms: i64,
}

/// 单次部署的最终结果记录。
#[derive(Clone, Debug, Deserialize, Serialize)]
pub(super) struct DeploymentRecord {
    pub(super) release: String,
    pub(super) previous_release: Option<String>,
    pub(super) outcome: DeploymentOutcome,
    pub(super) message: Option<String>,
    pub(super) recorded_at_ms: i64,
}

/// 部署完成后可恢复的稳定结果类别。
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum DeploymentOutcome {
    Succeeded,
    FailedRolledBack,
    FailedRollbackFailed,
}

/// 每个全托管 Service 的本地状态清单。
#[derive(Debug, Deserialize, Serialize)]
pub(super) struct ManagedState {
    pub(super) project: String,
    pub(super) active_release: Option<String>,
    /// 已落盘但尚未确认完成或回滚的 release。
    #[serde(default)]
    pub(super) pending_release: Option<String>,
    #[serde(default)]
    pub(super) releases: Vec<ReleaseRecord>,
    #[serde(default)]
    pub(super) deployments: Vec<DeploymentRecord>,
}

impl ManagedState {
    /// 读取已有状态或创建空清单。
    pub(super) fn load(path: &Path, project: &str) -> anyhow::Result<Self> {
        if !path.exists() {
            return Ok(Self {
                project: project.to_owned(),
                active_release: None,
                pending_release: None,
                releases: Vec::new(),
                deployments: Vec::new(),
            });
        }
        let bytes = fs::read(path).with_context(|| format!("无法读取 `{}`", path.display()))?;
        let state: Self = serde_json::from_slice(&bytes)
            .with_context(|| format!("托管状态 `{}` 无效", path.display()))?;
        if state.project != project {
            bail!(
                "托管目录属于 Service `{}`，不能用于 `{project}`",
                state.project
            );
        }
        Ok(state)
    }

    /// 原子保存状态清单。
    pub(super) fn save(&self, path: &Path) -> anyhow::Result<()> {
        let parent = path.parent().context("托管状态文件没有父目录")?;
        fs::create_dir_all(parent)?;
        let temporary = parent.join(format!(".state-{}.json", uuid::Uuid::new_v4()));
        let result = (|| -> anyhow::Result<()> {
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)?;
            serde_json::to_writer_pretty(&mut file, self)?;
            file.write_all(b"\n")?;
            file.sync_all()?;
            #[cfg(windows)]
            if path.exists() {
                fs::remove_file(path)?;
            }
            fs::rename(&temporary, path)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(temporary);
        }
        result
    }

    /// 登记尚未出现过的 release。
    pub(super) fn register_release(
        &mut self,
        id: &str,
        sha256: &str,
        config_path: &Path,
    ) -> anyhow::Result<()> {
        if let Some(existing) = self.releases.iter().find(|release| release.id == id) {
            if existing.sha256 != sha256 || existing.config_path != config_path {
                bail!("release ID `{id}` 与已有内容冲突，拒绝覆盖不可变版本");
            }
            return Ok(());
        }
        self.releases.push(ReleaseRecord {
            id: id.to_owned(),
            sha256: sha256.to_owned(),
            config_path: config_path.to_path_buf(),
            deployed_at_ms: now_millis(),
        });
        Ok(())
    }

    /// 返回指定 release 保存的配置入口。
    pub(super) fn config_path(&self, id: &str) -> Option<&Path> {
        self.releases
            .iter()
            .find(|release| release.id == id)
            .map(|release| release.config_path.as_path())
    }

    /// 追加有界部署历史。
    pub(super) fn record(&mut self, record: DeploymentRecord) {
        self.deployments.push(record);
        if self.deployments.len() > 100 {
            self.deployments
                .drain(..self.deployments.len().saturating_sub(100));
        }
    }

    /// 从清单移除非活动的过期release，并返回提交后可安全删除的目录名。
    pub(super) fn prune(&mut self, keep: usize) -> Vec<String> {
        let mut removed = Vec::new();
        self.releases.sort_by_key(|release| release.deployed_at_ms);
        while self.releases.len() > keep {
            let removable = self
                .releases
                .iter()
                .position(|release| Some(&release.id) != self.active_release.as_ref());
            let Some(index) = removable else {
                break;
            };
            let release = self.releases.remove(index);
            removed.push(release.id);
        }
        removed
    }
}

/// 返回当前 Unix 纪元毫秒数。
pub(super) fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(i64::MAX)
}

/// 返回 release 在托管目录中的完整路径。
pub(super) fn release_path(releases_root: &Path, release: &str) -> PathBuf {
    releases_root.join(release)
}
