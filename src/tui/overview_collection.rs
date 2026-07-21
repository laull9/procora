use std::cmp::Ordering;

use crate::protocol::{ResourceUsageDto, ServiceStatusDto, ServiceViewDto};

/// 总览支持的服务排序字段。
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum OverviewSort {
    /// 按服务稳定名称排序。
    #[default]
    Name,
    /// 按运行状态排序。
    Status,
    /// 按 Service 聚合 CPU 排序。
    Cpu,
    /// 按 Service 聚合内存排序。
    Memory,
}

impl OverviewSort {
    /// 切换到下一个排序字段。
    pub(super) const fn next(self) -> Self {
        match self {
            Self::Name => Self::Status,
            Self::Status => Self::Cpu,
            Self::Cpu => Self::Memory,
            Self::Memory => Self::Name,
        }
    }

    /// 返回排序字段的中文短标签。
    pub const fn label(self) -> &'static str {
        match self {
            Self::Name => "名称",
            Self::Status => "状态",
            Self::Cpu => "CPU",
            Self::Memory => "内存",
        }
    }

    /// 返回该字段更适合的默认排序方向。
    pub(super) const fn default_descending(self) -> bool {
        matches!(self, Self::Cpu | Self::Memory)
    }
}

/// 按当前筛选和排序生成总览可见服务列表。
pub(super) fn visible_services(
    services: &[ServiceViewDto],
    query: &str,
    sort: OverviewSort,
    descending: bool,
) -> Vec<ServiceViewDto> {
    let query = query.trim().to_lowercase();
    let mut visible = services
        .iter()
        .filter(|service| query.is_empty() || matches_query(service, &query))
        .cloned()
        .collect::<Vec<_>>();
    visible.sort_by(|left, right| {
        let ordering = compare_service(left, right, sort);
        let ordering = if descending {
            ordering.reverse()
        } else {
            ordering
        };
        ordering.then_with(|| left.name.cmp(&right.name))
    });
    visible
}

/// 聚合当前可见 Service 的资源占用。
pub(super) fn aggregate_resources(services: &[ServiceViewDto]) -> Option<ResourceUsageDto> {
    let mut cpu = None;
    let mut memory = None;
    for resources in services.iter().filter_map(|service| service.resources) {
        if let Some(value) = resources.cpu_tenths_percent {
            cpu = Some(cpu.unwrap_or(0_u16).saturating_add(value).min(1000_u16));
        }
        if let Some(value) = resources.memory_bytes {
            memory = Some(memory.unwrap_or(0_u64).saturating_add(value));
        }
    }
    (cpu.is_some() || memory.is_some()).then_some(ResourceUsageDto {
        cpu_tenths_percent: cpu,
        memory_bytes: memory,
    })
}

/// 判断服务的常用可见字段是否命中大小写不敏感查询。
fn matches_query(service: &ServiceViewDto, query: &str) -> bool {
    let status = match service.status {
        ServiceStatusDto::Running => "running 运行中",
        ServiceStatusDto::Stopped => "stopped 已停止",
        ServiceStatusDto::Failed => "failed 失败",
    };
    service.name.to_lowercase().contains(query)
        || service
            .root
            .to_string_lossy()
            .to_lowercase()
            .contains(query)
        || service
            .config_path
            .to_string_lossy()
            .to_lowercase()
            .contains(query)
        || service
            .message
            .as_deref()
            .is_some_and(|message| message.to_lowercase().contains(query))
        || status.contains(query)
}

/// 按指定字段比较两个服务摘要。
fn compare_service(left: &ServiceViewDto, right: &ServiceViewDto, sort: OverviewSort) -> Ordering {
    match sort {
        OverviewSort::Name => left.name.cmp(&right.name),
        OverviewSort::Status => status_rank(left.status).cmp(&status_rank(right.status)),
        OverviewSort::Cpu => cpu_value(left).cmp(&cpu_value(right)),
        OverviewSort::Memory => memory_value(left).cmp(&memory_value(right)),
    }
}

/// 返回服务状态的稳定排序等级。
const fn status_rank(status: ServiceStatusDto) -> u8 {
    match status {
        ServiceStatusDto::Running => 0,
        ServiceStatusDto::Stopped => 1,
        ServiceStatusDto::Failed => 2,
    }
}

/// 返回用于排序的可选 CPU 值。
fn cpu_value(service: &ServiceViewDto) -> Option<u16> {
    service
        .resources
        .and_then(|resources| resources.cpu_tenths_percent)
}

/// 返回用于排序的可选内存值。
fn memory_value(service: &ServiceViewDto) -> Option<u64> {
    service
        .resources
        .and_then(|resources| resources.memory_bytes)
}
