use std::{
    io::{self, Write},
    str::FromStr,
};

use anyhow::{Context, bail};

use crate::{
    core::TaskId,
    protocol::{CenterRequest, CenterResponse, LOG_STREAM_CHUNK_BYTES},
};

use super::{api, center_runtime};

/// 流式读取、匹配或清空指定服务的 Task 文件日志。
pub(super) fn run(
    target: &str,
    task: &str,
    search: Option<&str>,
    filter: Option<&str>,
    clear: bool,
) -> anyhow::Result<()> {
    let task_id = TaskId::from_str(task).with_context(|| format!("无效 Task 标识：{task}"))?;
    let client = center_runtime::running_center()?.context("全局 Procora 服务器未运行")?;
    let selector = api::selector(target)?;
    if clear {
        return match client.request(&CenterRequest::ClearTaskLogs {
            selector,
            task_id: task_id.clone(),
        })? {
            CenterResponse::TaskLogsCleared(cleared) if cleared == task_id => {
                println!("已清空 Task `{task_id}` 的日志");
                Ok(())
            }
            CenterResponse::Error { message } => bail!(message),
            response => bail!("全局 Procora 服务器返回了意外响应: {response:?}"),
        };
    }

    let stdout = io::stdout();
    let mut output = io::BufWriter::new(stdout.lock());
    let mut filter = LineFilter::new(search, filter);
    let mut cursor = None;
    loop {
        let batch = client.read_task_logs(&selector, &task_id, cursor)?;
        if batch.gap {
            eprintln!("警告：日志读取期间发生轮转，输出已从当前可用位置恢复");
        }
        let length = batch.bytes.len();
        cursor = Some(batch.next_cursor);
        filter.write(&batch.bytes, &mut output)?;
        output.flush()?;
        if length < LOG_STREAM_CHUNK_BYTES as usize {
            break;
        }
    }
    filter.finish(&mut output)?;
    output.flush()?;
    Ok(())
}

/// 跨日志分片保留半行并执行搜索或过滤。
struct LineFilter {
    query: Option<String>,
    numbered: bool,
    pending: Vec<u8>,
    line_number: usize,
}

impl LineFilter {
    /// 创建原样输出、带行号搜索或无行号过滤模式。
    fn new(search: Option<&str>, filter: Option<&str>) -> Self {
        Self {
            query: search.or(filter).map(str::to_ascii_lowercase),
            numbered: search.is_some(),
            pending: Vec::new(),
            line_number: 1,
        }
    }

    /// 消费一个日志分片，只有完整行才立即进入文本匹配。
    fn write(&mut self, bytes: &[u8], output: &mut impl Write) -> io::Result<()> {
        if self.query.is_none() {
            return output.write_all(bytes);
        }
        self.pending.extend_from_slice(bytes);
        while let Some(end) = self.pending.iter().position(|byte| *byte == b'\n') {
            let line = self.pending.drain(..=end).collect::<Vec<_>>();
            self.write_line(&line[..line.len() - 1], output)?;
        }
        Ok(())
    }

    /// 输出最后一个没有换行符的日志行。
    fn finish(&mut self, output: &mut impl Write) -> io::Result<()> {
        if self.query.is_some() && !self.pending.is_empty() {
            let line = std::mem::take(&mut self.pending);
            self.write_line(&line, output)?;
        }
        Ok(())
    }

    /// 匹配并输出一条已经去除换行符的日志行。
    fn write_line(&mut self, bytes: &[u8], output: &mut impl Write) -> io::Result<()> {
        let line = String::from_utf8_lossy(bytes);
        let searchable = crate::log::strip_ansi(&line).to_ascii_lowercase();
        if self
            .query
            .as_ref()
            .is_some_and(|query| searchable.contains(query))
        {
            if self.numbered {
                writeln!(output, "{}:{line}", self.line_number)?;
            } else {
                writeln!(output, "{line}")?;
            }
        }
        self.line_number += 1;
        Ok(())
    }
}
