//! TUI 页面内容的短时滑入转场状态。

use std::time::Duration;

use ratatui::layout::Rect;

/// 单次页面转场的总时长。
const TRANSITION_DURATION: Duration = Duration::from_millis(150);
/// 宽屏转场最多移动的终端列数。
const MAX_OFFSET: u16 = 3;

/// 新页面进入视口的方向。
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum TransitionDirection {
    /// 新页面从右侧进入。
    #[default]
    Forward,
    /// 新页面从左侧进入。
    Backward,
}

/// 可由事件循环按真实时间推进的页面转场。
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct UiTransition {
    direction: TransitionDirection,
    remaining: Duration,
}

impl UiTransition {
    /// 从指定方向重新开始一次转场。
    pub(crate) const fn start(&mut self, direction: TransitionDirection) {
        self.direction = direction;
        self.remaining = TRANSITION_DURATION;
    }

    /// 推进转场，并返回当前帧是否发生变化。
    pub(crate) fn advance(&mut self, elapsed: Duration) -> bool {
        if self.remaining.is_zero() {
            return false;
        }
        self.remaining = self.remaining.saturating_sub(elapsed);
        true
    }

    /// 返回当前帧内容应使用的滑入区域。
    pub(crate) fn content_area(self, area: Rect) -> Rect {
        let offset = self.offset(area.width);
        if offset == 0 {
            return area;
        }
        match self.direction {
            TransitionDirection::Forward => Rect {
                x: area.x.saturating_add(offset),
                width: area.width.saturating_sub(offset),
                ..area
            },
            TransitionDirection::Backward => Rect {
                width: area.width.saturating_sub(offset),
                ..area
            },
        }
    }

    /// 返回当前转场是否尚未结束。
    pub(crate) const fn active(self) -> bool {
        !self.remaining.is_zero()
    }

    /// 根据剩余时间计算当前帧偏移。
    fn offset(self, width: u16) -> u16 {
        if self.remaining.is_zero() {
            return 0;
        }
        let maximum = if width < 48 { 1 } else { MAX_OFFSET };
        let remaining = self.remaining.as_millis();
        let duration = TRANSITION_DURATION.as_millis();
        let scaled = remaining
            .saturating_mul(u128::from(maximum))
            .div_ceil(duration);
        u16::try_from(scaled).unwrap_or(maximum).min(maximum)
    }
}
