//! TUI 单行文本的宽度计算、折叠和水平视口辅助。

use std::time::Duration;

use ratatui::text::Line;

/// 折叠文本自动滚动的恒定阅读速度。
pub(crate) const AUTO_SCROLL_CHARS_PER_SECOND: u128 = 4;
/// 手动横移后当前高亮文本不受自动滚动影响的时长。
const MANUAL_SCROLL_FREEZE: Duration = Duration::from_secs(10);

/// 折叠文本全局自动滚动的开关状态。
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum AutoScrollMode {
    /// 保持手动水平移动。
    #[default]
    Off,
    /// 按恒定阅读速度自动水平移动。
    On,
}

impl AutoScrollMode {
    /// 返回自动滚动是否开启。
    pub(crate) const fn enabled(self) -> bool {
        matches!(self, Self::On)
    }

    /// 切换自动滚动状态。
    pub(crate) const fn toggle(&mut self) {
        *self = match self {
            Self::Off => Self::On,
            Self::On => Self::Off,
        };
    }
}

/// 共享的手动与全局自动水平滚动状态。
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct HorizontalScroll {
    manual_offset: usize,
    auto_mode: AutoScrollMode,
    auto_offset: usize,
    auto_remainder: Duration,
    manual_freeze: Duration,
}

impl HorizontalScroll {
    /// 返回手动水平偏移。
    pub(crate) const fn manual_offset(self) -> usize {
        self.manual_offset
    }

    /// 返回自动滚动是否开启。
    pub(crate) const fn auto_enabled(self) -> bool {
        self.auto_mode.enabled()
    }

    /// 返回当前高亮文本是否处于手动冻结期。
    pub(crate) const fn manual_frozen(self) -> bool {
        !self.manual_freeze.is_zero()
    }

    /// 返回一段文本在手动或全局自动模式下应使用的偏移。
    pub(crate) const fn offset(self, selected: bool) -> usize {
        if self.auto_enabled() && (!selected || !self.manual_frozen()) {
            self.auto_offset
        } else if selected {
            self.manual_offset
        } else {
            0
        }
    }

    /// 返回非选中文本仅在自动模式下使用的偏移。
    pub(crate) const fn automatic_offset(self) -> usize {
        if self.auto_enabled() {
            self.auto_offset
        } else {
            0
        }
    }

    /// 手动移动一次，并从当前自动位置开始十秒冻结。
    pub(crate) fn scroll_manual(&mut self, forward: bool, maximum: usize) {
        if self.auto_enabled() && !self.manual_frozen() {
            self.manual_offset = self.auto_offset.min(maximum);
        }
        self.manual_offset = if forward {
            self.manual_offset.saturating_add(1).min(maximum)
        } else {
            self.manual_offset.saturating_sub(1)
        };
        self.manual_freeze = MANUAL_SCROLL_FREEZE;
    }

    /// 推进一次按真实时间计算的恒速全局自动滚动。
    pub(crate) fn advance(&mut self, elapsed: Duration, maximum: usize) -> bool {
        let was_frozen = self.manual_frozen();
        self.manual_freeze = self.manual_freeze.saturating_sub(elapsed);
        if !self.auto_enabled() {
            return false;
        }
        let freeze_ended = was_frozen && !self.manual_frozen();
        let steps = auto_scroll_steps(&mut self.auto_remainder, elapsed);
        if steps == 0 {
            return freeze_ended;
        }
        if maximum == 0 {
            return false;
        }
        self.auto_offset = self.auto_offset.saturating_add(steps) % maximum.saturating_add(1);
        true
    }

    /// 切换自动模式并从文本起点重新开始。
    pub(crate) const fn toggle_auto(&mut self) {
        self.auto_mode.toggle();
        self.reset_position();
    }

    /// 重置当前位置与冻结计时，保留自动模式开关。
    pub(crate) const fn reset_position(&mut self) {
        self.manual_offset = 0;
        self.auto_offset = 0;
        self.auto_remainder = Duration::ZERO;
        self.manual_freeze = Duration::ZERO;
    }
}

/// 单行输入框内可见的文本与光标列。
pub(crate) struct InputView {
    /// 已按终端宽度折叠的文本。
    pub(crate) text: String,
    /// 光标相对可见文本起点的显示列。
    pub(crate) cursor_x: usize,
}

/// 返回文本占用的终端显示列数。
pub(crate) fn width(text: &str) -> usize {
    Line::from(text).width()
}

/// 从字符偏移处显示单行文本，并在被截断的方向放置省略号。
pub(crate) fn clipped(text: &str, offset: usize, max_width: usize) -> String {
    if max_width == 0 {
        return String::new();
    }
    let text_width = width(text);
    let offset = overflowing_offset(text_width, offset, max_width);
    if text_width <= max_width {
        return text.to_owned();
    }
    let characters = text.chars().collect::<Vec<_>>();
    let start = offset.min(characters.len());
    let prefix = usize::from(start > 0);
    let mut result = if prefix == 1 {
        "…".to_owned()
    } else {
        String::new()
    };
    let mut used = prefix;
    let mut end = start;
    while end < characters.len() {
        let character = characters[end];
        let character_width = width(&character.to_string());
        let needs_suffix = end + 1 < characters.len();
        let reserved = usize::from(needs_suffix);
        if used
            .saturating_add(character_width)
            .saturating_add(reserved)
            > max_width
        {
            break;
        }
        result.push(character);
        used += character_width;
        end += 1;
    }
    if end < characters.len() && used < max_width {
        result.push('…');
    }
    result
}

/// 只有文本真实超过可见宽度时才允许手动或自动偏移生效。
const fn overflowing_offset(text_width: usize, offset: usize, max_width: usize) -> usize {
    if text_width > max_width { offset } else { 0 }
}

/// 按真实经过时间换算自动滚动字符数，并保留不足一个字符的时间余量。
pub(crate) fn auto_scroll_steps(remainder: &mut Duration, elapsed: Duration) -> usize {
    let nanos_per_character = 1_000_000_000_u128 / AUTO_SCROLL_CHARS_PER_SECOND;
    let total = remainder.as_nanos().saturating_add(elapsed.as_nanos());
    let steps = total / nanos_per_character;
    let remaining = total % nanos_per_character;
    *remainder = Duration::from_nanos(u64::try_from(remaining).unwrap_or(u64::MAX));
    usize::try_from(steps).unwrap_or(usize::MAX)
}

/// 生成始终包含指定字符光标的单行输入视口。
pub(crate) fn input_view(text: &str, cursor: usize, max_width: usize) -> InputView {
    if max_width == 0 {
        return InputView {
            text: String::new(),
            cursor_x: 0,
        };
    }
    let characters = text.chars().collect::<Vec<_>>();
    let cursor = cursor.min(characters.len());
    let mut start = 0;
    while start < cursor {
        let prefix = usize::from(start > 0);
        let cursor_width = characters[start..cursor]
            .iter()
            .map(|character| width(&character.to_string()))
            .sum::<usize>();
        if prefix.saturating_add(cursor_width) < max_width {
            break;
        }
        start += 1;
    }
    let text = clipped(text, start, max_width);
    let prefix = usize::from(start > 0);
    let cursor_x = prefix
        + characters[start..cursor]
            .iter()
            .map(|character| width(&character.to_string()))
            .sum::<usize>();
    InputView {
        text,
        cursor_x: cursor_x.min(max_width.saturating_sub(1)),
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{auto_scroll_steps, clipped};

    #[test]
    // 未溢出的文本不受水平偏移影响。
    fn short_text_ignores_horizontal_offset() {
        assert_eq!(clipped("short", 3, 10), "short");
    }

    #[test]
    // 自动滚动按累计真实时间保持每秒四个字符的恒定速度。
    fn auto_scroll_speed_uses_elapsed_time() {
        let mut remainder = Duration::ZERO;

        assert_eq!(
            auto_scroll_steps(&mut remainder, Duration::from_millis(625)),
            2
        );
        assert_eq!(
            auto_scroll_steps(&mut remainder, Duration::from_millis(375)),
            2
        );
        assert_eq!(remainder, Duration::ZERO);
    }
}
