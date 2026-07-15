//! 任务日志帧、游标与内存尾部缓冲。

mod file_store;

use std::collections::VecDeque;

use procora_core::TaskId;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub use file_store::{FileLogBatch, FileLogCursor, FileLogError, FileLogPolicy, FileLogStore};

/// 日志来源流。
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LogStream {
    /// 子进程标准输出。
    Stdout,
    /// 子进程标准错误。
    Stderr,
    /// Procora 生成的系统消息。
    System,
}

/// 带运行身份和顺序号的日志帧。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LogFrame {
    /// 任务稳定标识。
    pub task_id: TaskId,
    /// 本次进程运行标识。
    pub run_id: Uuid,
    /// 本次运行内单调递增的序号。
    pub sequence: u64,
    /// 日志来源流。
    pub stream: LogStream,
    /// 未假设文本编码的原始内容。
    pub bytes: Vec<u8>,
}

/// 保留最近若干日志帧的有界内存缓冲。
#[derive(Debug)]
pub struct TailBuffer {
    capacity: usize,
    frames: VecDeque<LogFrame>,
}

impl TailBuffer {
    /// 创建指定帧容量的尾部缓冲。
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            frames: VecDeque::with_capacity(capacity),
        }
    }

    /// 追加日志帧并在超限时淘汰最旧内容。
    pub fn push(&mut self, frame: LogFrame) {
        if self.capacity == 0 {
            return;
        }
        if self.frames.len() == self.capacity {
            self.frames.pop_front();
        }
        self.frames.push_back(frame);
    }

    /// 按存储顺序访问当前日志帧。
    pub fn iter(&self) -> impl Iterator<Item = &LogFrame> {
        self.frames.iter()
    }
}
