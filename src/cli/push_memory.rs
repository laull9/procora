use std::{
    fs,
    io::{self, Read, Write},
    path::PathBuf,
};

use anyhow::{Context, bail};
use serde::{Deserialize, Serialize};

use super::{center_runtime, push::SourceMethod};

/// 上一次 push 引导的非敏感选择。
#[derive(Debug, Default, Deserialize, Serialize)]
pub(super) struct PushMemory {
    pub(super) source_method: SourceMethod,
    pub(super) source: Option<PathBuf>,
    pub(super) ssh_target: Option<String>,
    pub(super) remote_bin: Option<String>,
    pub(super) upload_target: Option<String>,
    pub(super) restart: bool,
}

/// 返回全局 Procora 内的 push 记忆文件。
fn memory_path() -> anyhow::Result<PathBuf> {
    Ok(center_runtime::center_paths()?
        .home
        .join("cli-memory")
        .join("push.json"))
}

/// 读取上次成功 push 的非敏感交互选择。
pub(super) fn load_memory() -> PushMemory {
    let result = (|| -> anyhow::Result<PushMemory> {
        let path = memory_path()?;
        let mut file = match fs::File::open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Ok(PushMemory::default());
            }
            Err(error) => return Err(error.into()),
        };
        let mut bytes = Vec::new();
        Read::by_ref(&mut file)
            .take(64 * 1024 + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() > 64 * 1024 {
            bail!("记忆文件超过 64 KiB");
        }
        let mut memory: PushMemory =
            serde_json::from_slice(&bytes).context("push 记忆文件不是有效 JSON")?;
        memory.source = memory.source.as_deref().map(crate::platform::simplify_path);
        Ok(memory)
    })();
    result.unwrap_or_else(|error| {
        eprintln!("警告：无法读取 push 引导记忆，将使用默认值：{error:#}");
        PushMemory::default()
    })
}

/// 原子保存上次成功 push 的非敏感交互选择。
pub(super) fn save_memory(memory: &PushMemory) -> anyhow::Result<()> {
    let path = memory_path()?;
    let parent = path.parent().context("push 记忆文件没有父目录")?;
    fs::create_dir_all(parent)?;
    let temporary = parent.join(format!(".push-{}.json", uuid::Uuid::new_v4()));
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
            fs::remove_file(&path)?;
        }
        fs::rename(&temporary, &path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}
