//! One scan: run every provider, keep what each one could determine and
//! summarize how complete the result is.
//!
//! The scan is read-only and needs no privileges. Providers never abort each
//! other: an unavailable subsystem becomes a diagnostic and the report keeps the
//! rest, which is what lets the runtime degrade instead of guessing.

use crate::model::{
    Diagnostic, DiagnosticLevel, HardwareReport, Provider, SCHEMA_VERSION, ScanStatus,
};
use crate::source::{HostSource, LinuxHost};
use crate::{cpu, display, gpu, memory, system};

/// Scans the host described by `source`.
pub fn scan(source: &dyn HostSource) -> HardwareReport {
    let system = system::detect(source);
    let cpu = cpu::detect(source);
    let memory = memory::detect(source);
    let gpus = gpu::detect(source);
    let displays = display::detect(source);

    let mut diagnostics = Vec::new();
    diagnostics.extend(system.diagnostics.iter().cloned());
    diagnostics.extend(cpu.diagnostics.iter().cloned());
    diagnostics.extend(memory.diagnostics.iter().cloned());
    diagnostics.extend(gpus.diagnostics.iter().cloned());
    diagnostics.extend(displays.diagnostics.iter().cloned());
    diagnostics.push(vulkan_not_consulted());

    let system = system.value.unwrap_or_else(unknown_system);
    let host_unknown =
        system.distribution.is_none() && system.kernel.is_none() && system.libc.is_none();
    // Any finding above informational level means some part of the host stayed
    // unknown, which is exactly what `Partial` reports.
    let degraded = cpu.has_failures()
        || memory.has_failures()
        || gpus.has_failures()
        || displays.has_failures()
        || diagnostics
            .iter()
            .any(|diagnostic| diagnostic.level != DiagnosticLevel::Info);
    let scan_status = if host_unknown {
        ScanStatus::Failed
    } else if degraded {
        ScanStatus::Partial
    } else {
        ScanStatus::Complete
    };

    HardwareReport {
        schema_version: SCHEMA_VERSION.to_string(),
        scan_status,
        system,
        cpu: cpu.value,
        gpus: gpus.value.unwrap_or_default(),
        memory: memory.value,
        displays: displays.value.unwrap_or_default(),
        diagnostics,
    }
}

/// Scans the machine this process is running on.
pub fn scan_current_host() -> HardwareReport {
    scan(&LinuxHost)
}

/// The identity of a host whose system provider could not answer at all.
///
/// The report still states the architecture: it is a property of the binary, not
/// of a successful discovery, and a consumer needs it to reject a report it
/// cannot interpret.
fn unknown_system() -> crate::model::SystemInfo {
    crate::model::SystemInfo {
        architecture: std::env::consts::ARCH.to_string(),
        kernel: None,
        distribution: None,
        libc: None,
    }
}

/// States the scanned scope, so no consumer mistakes silence for evidence.
fn vulkan_not_consulted() -> Diagnostic {
    Diagnostic {
        provider: Provider::Vulkan,
        level: DiagnosticLevel::Info,
        code: "not_consulted".to_string(),
        message: "Vulkan enumeration is not part of this scan; an absent Vulkan section does not invalidate the report"
            .to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{DiagnosticLevel, Provider, ScanStatus};
    use crate::source::MappingSource;

    fn complete_host() -> MappingSource {
        MappingSource::new()
            .with_file(
                "/etc/os-release",
                "ID=ubuntu\nNAME=\"Ubuntu\"\nVERSION_ID=\"24.04\"\nPRETTY_NAME=\"Ubuntu 24.04.1 LTS\"\n",
            )
            .with_file("/proc/sys/kernel/osrelease", "6.8.0-45-generic\n")
            .with_file(
                "/proc/cpuinfo",
                "processor\t: 0\nvendor_id\t: AuthenticAMD\nmodel name\t: AMD Ryzen 7 7840U\nphysical id\t: 0\ncpu cores\t: 8\nflags\t\t: fpu avx2\n",
            )
            .with_file("/proc/meminfo", "MemTotal: 32608376 kB\nMemAvailable: 20480000 kB\n")
            .with_present_path("/lib/x86_64-linux-gnu/libc.so.6")
            .with_dir("/sys/bus/pci/devices", ["0000:03:00.0"])
            .with_file("/sys/bus/pci/devices/0000:03:00.0/class", "0x030000\n")
            .with_file("/sys/bus/pci/devices/0000:03:00.0/vendor", "0x1002\n")
            .with_file("/sys/bus/pci/devices/0000:03:00.0/device", "0x15bf\n")
            .with_dir("/sys/class/drm", ["card0-eDP-1"])
            .with_file("/sys/class/drm/card0-eDP-1/status", "connected\n")
    }

    #[test]
    fn a_fully_readable_host_reports_complete() {
        let report = scan(&complete_host());
        assert_eq!(report.scan_status, ScanStatus::Complete);
        assert_eq!(report.schema_version, crate::model::SCHEMA_VERSION);
        assert_eq!(report.gpus.len(), 1);
        assert_eq!(report.displays.len(), 1);
        assert!(report.cpu.is_some());
        assert!(report.memory.is_some());
        assert!(report.system.libc.is_some());
    }

    #[test]
    fn the_scan_always_states_that_vulkan_was_not_consulted() {
        let report = scan(&complete_host());
        let vulkan = report
            .diagnostics
            .iter()
            .find(|d| d.provider == Provider::Vulkan)
            .expect("the report says Vulkan was not consulted");
        assert_eq!(vulkan.code, "not_consulted");
        assert_eq!(vulkan.level, DiagnosticLevel::Info);
        assert_eq!(
            report.scan_status,
            ScanStatus::Complete,
            "an absent Vulkan must never invalidate the report"
        );
    }

    #[test]
    fn a_failed_subsystem_degrades_the_scan_to_partial() {
        let mut source = complete_host();
        source = source
            .with_file("/proc/cpuinfo", "")
            .with_file("/sys/bus/pci/devices/0000:03:00.0/class", "0x020000\n");
        let report = scan(&source);
        assert_eq!(report.scan_status, ScanStatus::Partial);
        assert_eq!(report.cpu, None);
        assert!(
            report
                .diagnostics
                .iter()
                .any(|d| d.level == DiagnosticLevel::Error && d.provider == Provider::Cpu)
        );
    }

    #[test]
    fn a_host_without_any_system_fact_is_failed() {
        let report = scan(&MappingSource::new());
        assert_eq!(report.scan_status, ScanStatus::Failed);
        assert_eq!(report.system.distribution, None);
        assert_eq!(report.system.kernel, None);
        assert_eq!(report.system.libc, None);
        assert!(report.cpu.is_none());
        assert!(report.gpus.is_empty());
        assert!(report.displays.is_empty());
    }

    #[test]
    fn a_partial_report_serializes_without_nulls() {
        let report = scan(&MappingSource::new().with_file("/etc/os-release", "ID=arch\n"));
        let value = serde_json::to_value(&report).expect("the report serializes");
        assert_no_nulls(&value);
        assert_eq!(value["scan_status"], serde_json::json!("partial"));
    }

    fn assert_no_nulls(value: &serde_json::Value) {
        match value {
            serde_json::Value::Null => panic!("a report must never serialize a null: {value}"),
            serde_json::Value::Array(items) => items.iter().for_each(assert_no_nulls),
            serde_json::Value::Object(fields) => fields.values().for_each(assert_no_nulls),
            _ => {}
        }
    }
}
