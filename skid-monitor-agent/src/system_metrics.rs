//! Linux host/system metric collection.
//!
//! This module intentionally reads stable kernel interfaces directly instead of
//! depending on a heavier metrics crate. It keeps skid-monitor useful on small
//! nodes and in offline builds.

use skid_protocol::metrics::{Metric, MetricKind, Source};
use std::collections::HashSet;
use std::ffi::CString;
use std::fs;
use std::path::Path;

const PROC_STAT: &str = "/proc/stat";
const PROC_LOADAVG: &str = "/proc/loadavg";
const PROC_MEMINFO: &str = "/proc/meminfo";
const PROC_UPTIME: &str = "/proc/uptime";
const PROC_MOUNTS: &str = "/proc/mounts";
const PROC_DISKSTATS: &str = "/proc/diskstats";
const PROC_NET_DEV: &str = "/proc/net/dev";
const PROC_SELF_STATUS: &str = "/proc/self/status";
const PROC_SELF_FD: &str = "/proc/self/fd";
const SECTOR_BYTES: f64 = 512.0;

#[derive(Debug, Default)]
pub struct SystemSampler {
    previous_cpu: Option<CpuSnapshot>,
}

impl SystemSampler {
    pub fn new() -> Self {
        Self {
            previous_cpu: fs::read_to_string(PROC_STAT)
                .ok()
                .and_then(|stat| parse_cpu_snapshot(&stat)),
        }
    }

    pub fn collect(&mut self) -> Vec<Metric> {
        let mut out = Vec::new();

        if let Ok(stat) = fs::read_to_string(PROC_STAT) {
            let current_cpu = collect_cpu_metrics(&stat, self.previous_cpu.as_ref(), &mut out);
            self.previous_cpu = current_cpu;
        }
        if let Ok(loadavg) = fs::read_to_string(PROC_LOADAVG) {
            collect_load_metrics(&loadavg, &mut out);
        }
        if let Ok(meminfo) = fs::read_to_string(PROC_MEMINFO) {
            collect_memory_metrics(&meminfo, &mut out);
        }
        if let Ok(uptime) = fs::read_to_string(PROC_UPTIME) {
            collect_uptime_metrics(&uptime, &mut out);
        }
        if let Ok(mounts) = fs::read_to_string(PROC_MOUNTS) {
            collect_filesystem_metrics(&mounts, &mut out);
        }
        if let Ok(diskstats) = fs::read_to_string(PROC_DISKSTATS) {
            collect_disk_io_metrics(&diskstats, &mut out);
        }
        if let Ok(net_dev) = fs::read_to_string(PROC_NET_DEV) {
            collect_network_metrics(&net_dev, &mut out);
        }
        if let Ok(status) = fs::read_to_string(PROC_SELF_STATUS) {
            collect_process_metrics(&status, &mut out);
        }
        collect_open_fd_count(&mut out);

        out
    }
}

#[derive(Debug, Clone)]
struct CpuSnapshot {
    cpus: Vec<CpuLine>,
}

#[derive(Debug, Clone)]
struct CpuLine {
    name: String,
    total: u64,
    idle: u64,
}

fn collect_cpu_metrics(
    proc_stat: &str,
    previous: Option<&CpuSnapshot>,
    out: &mut Vec<Metric>,
) -> Option<CpuSnapshot> {
    let current = parse_cpu_snapshot(proc_stat)?;
    let previous = match previous {
        Some(previous) => previous,
        None => return Some(current),
    };

    for cpu in &current.cpus {
        let Some(prev) = previous.cpus.iter().find(|prev| prev.name == cpu.name) else {
            continue;
        };
        let total_delta = cpu.total.saturating_sub(prev.total);
        if total_delta == 0 {
            continue;
        }

        let idle_delta = cpu.idle.saturating_sub(prev.idle);
        let usage = 100.0 * (total_delta.saturating_sub(idle_delta) as f64) / (total_delta as f64);
        let cpu_attr = if cpu.name == "cpu" {
            "total".to_string()
        } else {
            cpu.name.trim_start_matches("cpu").to_string()
        };
        push_metric(
            out,
            "system.cpu.usage",
            usage,
            Some("%"),
            MetricKind::Gauge,
            vec![("cpu".to_string(), cpu_attr)],
        );
    }

    Some(current)
}

fn parse_cpu_snapshot(proc_stat: &str) -> Option<CpuSnapshot> {
    let cpus = proc_stat
        .lines()
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            let name = parts.next()?;
            if !name.starts_with("cpu") {
                return None;
            }

            let values = parts
                .map(str::parse::<u64>)
                .collect::<Result<Vec<_>, _>>()
                .ok()?;
            if values.len() < 4 {
                return None;
            }

            let idle = values.get(3).copied().unwrap_or(0) + values.get(4).copied().unwrap_or(0);
            let total = values.iter().sum();
            Some(CpuLine {
                name: name.to_string(),
                total,
                idle,
            })
        })
        .collect::<Vec<_>>();

    (!cpus.is_empty()).then_some(CpuSnapshot { cpus })
}

fn collect_load_metrics(loadavg: &str, out: &mut Vec<Metric>) {
    let values = loadavg
        .split_whitespace()
        .take(3)
        .filter_map(|value| value.parse::<f64>().ok())
        .collect::<Vec<_>>();
    for (name, value) in [
        ("system.load.1m", values.first().copied()),
        ("system.load.5m", values.get(1).copied()),
        ("system.load.15m", values.get(2).copied()),
    ] {
        if let Some(value) = value {
            push_metric(out, name, value, None, MetricKind::Gauge, Vec::new());
        }
    }
}

fn collect_memory_metrics(meminfo: &str, out: &mut Vec<Metric>) {
    let lookup = |key: &str| parse_meminfo_kib(meminfo, key).map(|kib| kib * 1024.0);

    let total = lookup("MemTotal");
    let available = lookup("MemAvailable");
    for (name, value) in [
        ("system.memory.total", total),
        ("system.memory.free", lookup("MemFree")),
        ("system.memory.available", available),
        ("system.memory.buffers", lookup("Buffers")),
        ("system.memory.cached", lookup("Cached")),
        ("system.swap.total", lookup("SwapTotal")),
        ("system.swap.free", lookup("SwapFree")),
    ] {
        if let Some(value) = value {
            push_metric(out, name, value, Some("By"), MetricKind::Gauge, Vec::new());
        }
    }

    if let (Some(total), Some(available)) = (total, available) {
        push_metric(
            out,
            "system.memory.used",
            total - available,
            Some("By"),
            MetricKind::Gauge,
            Vec::new(),
        );
        push_metric(
            out,
            "system.memory.usage",
            (total - available) * 100.0 / total,
            Some("%"),
            MetricKind::Gauge,
            Vec::new(),
        );
    }
}

fn parse_meminfo_kib(meminfo: &str, key: &str) -> Option<f64> {
    meminfo.lines().find_map(|line| {
        let (line_key, rest) = line.split_once(':')?;
        (line_key == key).then(|| {
            rest.split_whitespace()
                .next()
                .and_then(|value| value.parse::<f64>().ok())
        })?
    })
}

fn collect_uptime_metrics(uptime: &str, out: &mut Vec<Metric>) {
    if let Some(seconds) = uptime
        .split_whitespace()
        .next()
        .and_then(|value| value.parse::<f64>().ok())
    {
        push_metric(
            out,
            "system.uptime",
            seconds,
            Some("s"),
            MetricKind::Gauge,
            Vec::new(),
        );
    }
}

fn collect_filesystem_metrics(mounts: &str, out: &mut Vec<Metric>) {
    let mut seen = HashSet::new();
    for line in mounts.lines() {
        let parts = line.split_whitespace().collect::<Vec<_>>();
        if parts.len() < 3 {
            continue;
        }
        let device = unescape_mount_value(parts[0]);
        let mountpoint = unescape_mount_value(parts[1]);
        let fs_type = parts[2];
        if is_virtual_filesystem(fs_type) || !seen.insert(mountpoint.clone()) {
            continue;
        }

        let Some(stats) = statvfs(Path::new(&mountpoint)) else {
            continue;
        };
        let attrs = vec![
            ("device".to_string(), device),
            ("mountpoint".to_string(), mountpoint),
            ("fs_type".to_string(), fs_type.to_string()),
        ];
        push_metric(
            out,
            "system.filesystem.total",
            stats.total,
            Some("By"),
            MetricKind::Gauge,
            attrs.clone(),
        );
        push_metric(
            out,
            "system.filesystem.available",
            stats.available,
            Some("By"),
            MetricKind::Gauge,
            attrs.clone(),
        );
        push_metric(
            out,
            "system.filesystem.used",
            stats.used,
            Some("By"),
            MetricKind::Gauge,
            attrs.clone(),
        );
        if stats.total > 0.0 {
            push_metric(
                out,
                "system.filesystem.usage",
                stats.used * 100.0 / stats.total,
                Some("%"),
                MetricKind::Gauge,
                attrs,
            );
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct FilesystemStats {
    total: f64,
    available: f64,
    used: f64,
}

fn statvfs(path: &Path) -> Option<FilesystemStats> {
    let path = CString::new(path.as_os_str().as_encoded_bytes().to_vec()).ok()?;
    let mut stats = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    let result = unsafe { libc::statvfs(path.as_ptr(), stats.as_mut_ptr()) };
    if result != 0 {
        return None;
    }
    let stats = unsafe { stats.assume_init() };
    let block_size = stats.f_frsize as f64;
    let total = stats.f_blocks as f64 * block_size;
    let available = stats.f_bavail as f64 * block_size;
    let free = stats.f_bfree as f64 * block_size;
    Some(FilesystemStats {
        total,
        available,
        used: total - free,
    })
}

fn is_virtual_filesystem(fs_type: &str) -> bool {
    matches!(
        fs_type,
        "autofs"
            | "binfmt_misc"
            | "bpf"
            | "cgroup"
            | "cgroup2"
            | "configfs"
            | "debugfs"
            | "devpts"
            | "devtmpfs"
            | "fusectl"
            | "hugetlbfs"
            | "mqueue"
            | "nsfs"
            | "proc"
            | "pstore"
            | "securityfs"
            | "sysfs"
            | "tracefs"
    )
}

fn unescape_mount_value(value: &str) -> String {
    value
        .replace("\\040", " ")
        .replace("\\011", "\t")
        .replace("\\012", "\n")
        .replace("\\134", "\\")
}

fn collect_disk_io_metrics(diskstats: &str, out: &mut Vec<Metric>) {
    for line in diskstats.lines() {
        let parts = line.split_whitespace().collect::<Vec<_>>();
        if parts.len() < 14 {
            continue;
        }
        let device = parts[2];
        if device.starts_with("loop") || device.starts_with("ram") {
            continue;
        }

        let attrs = vec![("device".to_string(), device.to_string())];
        let read_ops = parse_f64(parts[3]);
        let sectors_read = parse_f64(parts[5]);
        let read_time_ms = parse_f64(parts[6]);
        let write_ops = parse_f64(parts[7]);
        let sectors_written = parse_f64(parts[9]);
        let write_time_ms = parse_f64(parts[10]);
        let io_in_progress = parse_f64(parts[11]);
        let io_time_ms = parse_f64(parts[12]);

        push_optional_metric(
            out,
            "system.disk.reads",
            read_ops,
            None,
            MetricKind::Sum,
            attrs.clone(),
        );
        push_optional_metric(
            out,
            "system.disk.read_bytes",
            sectors_read.map(|value| value * SECTOR_BYTES),
            Some("By"),
            MetricKind::Sum,
            attrs.clone(),
        );
        push_optional_metric(
            out,
            "system.disk.read_time",
            read_time_ms,
            Some("ms"),
            MetricKind::Sum,
            attrs.clone(),
        );
        push_optional_metric(
            out,
            "system.disk.writes",
            write_ops,
            None,
            MetricKind::Sum,
            attrs.clone(),
        );
        push_optional_metric(
            out,
            "system.disk.write_bytes",
            sectors_written.map(|value| value * SECTOR_BYTES),
            Some("By"),
            MetricKind::Sum,
            attrs.clone(),
        );
        push_optional_metric(
            out,
            "system.disk.write_time",
            write_time_ms,
            Some("ms"),
            MetricKind::Sum,
            attrs.clone(),
        );
        push_optional_metric(
            out,
            "system.disk.io_in_progress",
            io_in_progress,
            None,
            MetricKind::Gauge,
            attrs.clone(),
        );
        push_optional_metric(
            out,
            "system.disk.io_time",
            io_time_ms,
            Some("ms"),
            MetricKind::Sum,
            attrs,
        );
    }
}

fn collect_network_metrics(net_dev: &str, out: &mut Vec<Metric>) {
    for line in net_dev.lines().skip(2) {
        let Some((iface, values)) = line.split_once(':') else {
            continue;
        };
        let iface = iface.trim();
        if iface.is_empty() {
            continue;
        }

        let values = values
            .split_whitespace()
            .filter_map(|value| value.parse::<f64>().ok())
            .collect::<Vec<_>>();
        if values.len() < 16 {
            continue;
        }
        let attrs = vec![("interface".to_string(), iface.to_string())];
        for (name, value, unit) in [
            ("system.network.rx_bytes", values[0], Some("By")),
            ("system.network.rx_packets", values[1], None),
            ("system.network.rx_errors", values[2], None),
            ("system.network.rx_dropped", values[3], None),
            ("system.network.tx_bytes", values[8], Some("By")),
            ("system.network.tx_packets", values[9], None),
            ("system.network.tx_errors", values[10], None),
            ("system.network.tx_dropped", values[11], None),
        ] {
            push_metric(out, name, value, unit, MetricKind::Sum, attrs.clone());
        }
    }
}

fn collect_process_metrics(status: &str, out: &mut Vec<Metric>) {
    for (name, key, unit) in [
        ("process.memory.virtual", "VmSize", Some("By")),
        ("process.memory.resident", "VmRSS", Some("By")),
        ("process.memory.data", "VmData", Some("By")),
        ("process.memory.stack", "VmStk", Some("By")),
    ] {
        if let Some(kib) = parse_status_value(status, key) {
            push_metric(out, name, kib * 1024.0, unit, MetricKind::Gauge, Vec::new());
        }
    }

    for (name, key) in [
        ("process.threads", "Threads"),
        (
            "process.context_switches.voluntary",
            "voluntary_ctxt_switches",
        ),
        (
            "process.context_switches.nonvoluntary",
            "nonvoluntary_ctxt_switches",
        ),
    ] {
        if let Some(value) = parse_status_value(status, key) {
            push_metric(out, name, value, None, MetricKind::Gauge, Vec::new());
        }
    }
}

fn parse_status_value(status: &str, key: &str) -> Option<f64> {
    status.lines().find_map(|line| {
        let (line_key, rest) = line.split_once(':')?;
        (line_key == key).then(|| {
            rest.split_whitespace()
                .next()
                .and_then(|value| value.parse::<f64>().ok())
        })?
    })
}

fn collect_open_fd_count(out: &mut Vec<Metric>) {
    if let Ok(entries) = fs::read_dir(PROC_SELF_FD) {
        push_metric(
            out,
            "process.open_fds",
            entries.count() as f64,
            None,
            MetricKind::Gauge,
            Vec::new(),
        );
    }
}

fn parse_f64(value: &str) -> Option<f64> {
    value.parse::<f64>().ok()
}

fn push_optional_metric(
    out: &mut Vec<Metric>,
    name: &str,
    value: Option<f64>,
    unit: Option<&str>,
    kind: MetricKind,
    attributes: Vec<(String, String)>,
) {
    if let Some(value) = value {
        push_metric(out, name, value, unit, kind, attributes);
    }
}

fn push_metric(
    out: &mut Vec<Metric>,
    name: &str,
    value: f64,
    unit: Option<&str>,
    kind: MetricKind,
    attributes: Vec<(String, String)>,
) {
    out.push(Metric {
        name: name.to_string(),
        value,
        source: Source::System,
        unit: unit.map(str::to_string),
        kind,
        attributes,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn computes_cpu_usage_from_two_snapshots() {
        let first = "cpu  100 0 100 800 0 0 0 0 0 0\ncpu0 50 0 50 400 0 0 0 0 0 0\n";
        let second = "cpu  150 0 150 900 0 0 0 0 0 0\ncpu0 75 0 75 450 0 0 0 0 0 0\n";
        let previous = parse_cpu_snapshot(first).unwrap();
        let mut out = Vec::new();

        collect_cpu_metrics(second, Some(&previous), &mut out);

        let total = out
            .iter()
            .find(|metric| {
                metric
                    .attributes
                    .contains(&("cpu".to_string(), "total".to_string()))
            })
            .unwrap();
        assert_eq!(total.name, "system.cpu.usage");
        assert!((total.value - 50.0).abs() < f64::EPSILON);
    }

    #[test]
    fn parses_memory_and_process_status() {
        let meminfo = "\
MemTotal:       1000 kB
MemFree:         100 kB
MemAvailable:    400 kB
Buffers:          10 kB
Cached:           20 kB
SwapTotal:       500 kB
SwapFree:        300 kB
";
        let status = "\
VmSize:        2048 kB
VmRSS:         1024 kB
Threads:          8
voluntary_ctxt_switches: 12
nonvoluntary_ctxt_switches: 3
";
        let mut out = Vec::new();

        collect_memory_metrics(meminfo, &mut out);
        collect_process_metrics(status, &mut out);

        assert!(
            out.iter()
                .any(|metric| metric.name == "system.memory.used" && metric.value == 614400.0)
        );
        assert!(
            out.iter()
                .any(|metric| metric.name == "process.threads" && metric.value == 8.0)
        );
    }

    #[test]
    fn parses_network_device_counters() {
        let net_dev = "\
Inter-|   Receive                                                |  Transmit
 face |bytes    packets errs drop fifo frame compressed multicast|bytes    packets errs drop fifo colls carrier compressed
  eth0: 100 2 3 4 0 0 0 0 200 5 6 7 0 0 0 0
";
        let mut out = Vec::new();

        collect_network_metrics(net_dev, &mut out);

        assert!(
            out.iter()
                .any(|metric| metric.name == "system.network.rx_bytes" && metric.value == 100.0)
        );
        assert!(
            out.iter()
                .any(|metric| metric.name == "system.network.tx_packets" && metric.value == 5.0)
        );
    }
}
