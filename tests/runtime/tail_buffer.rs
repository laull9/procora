//! 日志尾部缓冲的容量行为测试。

use std::str::FromStr;

use procora::core::TaskId;
use procora::log::{LogFrame, LogStream, TailBuffer};
use uuid::Uuid;

/// 创建带指定序号的测试日志帧。
fn frame(sequence: u64) -> LogFrame {
    LogFrame {
        task_id: TaskId::from_str("api").unwrap(),
        run_id: Uuid::nil(),
        sequence,
        stream: LogStream::Stdout,
        bytes: sequence.to_string().into_bytes(),
    }
}

#[test]
// 缓冲区只保留最新帧。
fn tail_buffer_keeps_latest_frames() {
    let mut buffer = TailBuffer::new(2);
    buffer.push(frame(1));
    buffer.push(frame(2));
    buffer.push(frame(3));

    assert_eq!(
        buffer.iter().map(|item| item.sequence).collect::<Vec<_>>(),
        vec![2, 3]
    );
}
