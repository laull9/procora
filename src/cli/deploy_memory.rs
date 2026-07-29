//! 按 Service 根目录保存最近一次成功裸机部署的非敏感目标。

use std::{
    fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
};

use anyhow::{Context, bail};
use serde::{Deserialize, Serialize};

use super::center_runtime;

/// 最多记住的不同 Service 数量。
const MAX_ENTRIES: usize = 64;
/// 记忆文件的读取上限。
const MAX_BYTES: u64 = 64 * 1024;

/// 单个本地 Service 最近一次成功部署目标。
#[derive(Clone, Debug, Deserialize, Serialize)]
pub(super) struct DeployTargetMemory {
    pub(super) root: PathBuf,
    pub(super) project: String,
    pub(super) ssh_target: String,
    pub(super) remote_bin: String,
}

/// 全部本地 Service 的有界部署目标记忆。
#[derive(Debug, Default, Deserialize, Serialize)]
struct DeployMemory {
    #[serde(default)]
    entries: Vec<DeployTargetMemory>,
}

/// 返回全局 Procora 内的部署记忆文件。
fn memory_path() -> anyhow::Result<PathBuf> {
    Ok(center_runtime::center_paths()?
        .home
        .join("cli-memory")
        .join("deploy.json"))
}

/// 读取指定 Service 最近一次成功使用的目标。
pub(super) fn load_target(root: &Path, project: &str) -> Option<DeployTargetMemory> {
    let result = (|| -> anyhow::Result<Option<DeployTargetMemory>> {
        let path = memory_path()?;
        let mut file = match fs::File::open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        let mut bytes = Vec::new();
        Read::by_ref(&mut file)
            .take(MAX_BYTES + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_BYTES {
            bail!("部署记忆文件超过 64 KiB");
        }
        let mut memory: DeployMemory =
            serde_json::from_slice(&bytes).context("部署记忆文件不是有效 JSON")?;
        if memory.entries.len() > MAX_ENTRIES {
            bail!("部署记忆条目超过 {MAX_ENTRIES} 个");
        }
        for entry in &mut memory.entries {
            entry.root = crate::platform::simplify_path(&entry.root);
        }
        Ok(memory
            .entries
            .into_iter()
            .find(|entry| entry.root == root && entry.project == project))
    })();
    result
        .map_err(|error| {
            eprintln!("警告：无法读取部署目标记忆，将重新选择目标：{error:#}");
        })
        .ok()
        .flatten()
}

/// 保存指定 Service 最近一次成功使用的目标。
pub(super) fn save_target(target: DeployTargetMemory) -> anyhow::Result<()> {
    let path = memory_path()?;
    let mut memory = load_all(&path)?;
    memory
        .entries
        .retain(|entry| entry.root != target.root && entry.project != target.project);
    memory.entries.push(target);
    if memory.entries.len() > MAX_ENTRIES {
        memory
            .entries
            .drain(..memory.entries.len().saturating_sub(MAX_ENTRIES));
    }
    save_all(&path, &memory)
}

/// 读取整个记忆文件，供成功部署后更新。
fn load_all(path: &Path) -> anyhow::Result<DeployMemory> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(DeployMemory::default());
        }
        Err(error) => return Err(error.into()),
    };
    if bytes.len() as u64 > MAX_BYTES {
        bail!("部署记忆文件超过 64 KiB");
    }
    let memory: DeployMemory =
        serde_json::from_slice(&bytes).context("部署记忆文件不是有效 JSON")?;
    if memory.entries.len() > MAX_ENTRIES {
        bail!("部署记忆条目超过 {MAX_ENTRIES} 个");
    }
    Ok(memory)
}

/// 以仅当前用户可读写的临时文件原子替换部署记忆。
fn save_all(path: &Path, memory: &DeployMemory) -> anyhow::Result<()> {
    let parent = path.parent().context("部署记忆文件没有父目录")?;
    fs::create_dir_all(parent)?;
    let temporary = parent.join(format!(".deploy-{}.json", uuid::Uuid::new_v4()));
    let result = (|| -> anyhow::Result<()> {
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary)?;
        serde_json::to_writer_pretty(&mut file, memory)?;
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
        let _ = fs::remove_file(&temporary);
    }
    result
}
