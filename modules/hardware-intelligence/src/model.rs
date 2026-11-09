//! Domain model of the hardware report.
//!
//! The report is the platform's public description of a host: the runtime reads
//! it to decide how to launch applications, so every field states what it means
//! and how it may be absent.
//!
//! Serialization rules, which consumers may rely on:
//!
//! * enums serialize as `snake_case` machine names, never as display text;
//! * an absent subsystem (`None`) is omitted from the JSON object instead of
//!   serialized as `null`, so a partial report stays small and unambiguous;
//! * collections are always present, possibly empty, so consumers can iterate
//!   without a null check;
//! * [`SCHEMA_VERSION`] identifies the shape; breaking edits bump it.

use serde::Serialize;

/// Version of the JSON shape emitted by [`HardwareReport`].
///
/// Additive fields keep the version; removing or repurposing a field bumps the
/// major number, so a consumer can refuse a report it does not understand.
pub const SCHEMA_VERSION: &str = "1.0";

/// The complete result of one scan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HardwareReport {
    /// Shape identifier; see [`SCHEMA_VERSION`].
    pub schema_version: String,
    /// How much of the report could be discovered.
    pub scan_status: ScanStatus,
    /// Host operating system identity. It is never `Option`: a scan that cannot
    /// describe the host at all reports [`ScanStatus::Failed`] with the reasons
    /// in `diagnostics`.
    pub system: SystemInfo,
    /// CPU details, absent when `/proc/cpuinfo` could not be read.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cpu: Option<CpuInfo>,
    /// GPUs discovered on the PCI bus, identified by their stable address.
    pub gpus: Vec<GpuInfo>,
    /// Physical memory, absent when `/proc/meminfo` could not be read.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory: Option<MemoryInfo>,
    /// Displays the kernel reports, possibly empty on a headless host.
    pub displays: Vec<DisplayInfo>,
    /// Per-subsystem findings, including the ones that degraded this report.
    pub diagnostics: Vec<Diagnostic>,
}

/// How much of the requested information this scan obtained.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ScanStatus {
    /// Every provider answered and no subsystem degraded.
    Complete,
    /// The host was described, but at least one subsystem is missing or failed.
    Partial,
    /// The host itself could not be described.
    Failed,
}

/// How well the platform supports a discovered component.
///
/// This scan reports [`SupportLevel::Unknown`] unless a later feature asserted
/// support: describing hardware never implies a support promise.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SupportLevel {
    /// Known to work with the platform.
    Supported,
    /// Works with documented limitations.
    Partial,
    /// Known not to work.
    Unsupported,
    /// No policy has been applied yet.
    Unknown,
}

/// Host operating system identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SystemInfo {
    /// Architecture the binary was built for, such as `x86_64` or `aarch64`.
    pub architecture: String,
    /// Kernel release from `/proc/sys/kernel/osrelease`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kernel: Option<String>,
    /// Distribution as declared by `/etc/os-release`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub distribution: Option<Distribution>,
    /// C library family and the evidence that identified it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub libc: Option<Libc>,
}

/// Distribution identity from `/etc/os-release`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Distribution {
    /// Machine identifier, such as `ubuntu` or `arch`.
    pub id: String,
    /// Human name, such as `Ubuntu`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Version identifier, such as `24.04`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version_id: Option<String>,
    /// Full pretty name as published by the distribution.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pretty_name: Option<String>,
    /// Compatible distribution identifiers, in published order.
    pub id_like: Vec<String>,
    /// Support policy for this distribution. Always [`SupportLevel::Unknown`]
    /// in this scan; the compatibility policy arrives with a later phase.
    pub support: SupportLevel,
}

/// C library family.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LibcFamily {
    /// GNU C Library.
    Glibc,
    /// musl libc.
    Musl,
    /// Neither family was identified.
    Unknown,
}

/// C library identification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Libc {
    /// Family that was identified.
    pub family: LibcFamily,
    /// Path that proved the identification, so the claim stays verifiable.
    pub evidence: String,
}

/// CPU description.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CpuInfo {
    /// Architecture the binary was built for.
    pub architecture: String,
    /// Vendor string, such as `GenuineIntel`, when the kernel publishes one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vendor: Option<String>,
    /// Model string, such as `AMD Ryzen 7 7840U w/ Radeon 780M Graphics`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Logical processors the kernel reports.
    pub logical_cores: u32,
    /// Physical cores, absent when the kernel does not publish the topology.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub physical_cores: Option<u32>,
    /// Physical packages (sockets).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sockets: Option<u32>,
    /// Sorted, de-duplicated CPU feature flags published by the kernel.
    pub features: Vec<String>,
}

/// Physical memory reported by `/proc/meminfo`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MemoryInfo {
    /// Total usable memory, in bytes.
    pub total_bytes: u64,
    /// Memory available for new allocations, in bytes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub available_bytes: Option<u64>,
    /// Total swap, in bytes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub swap_total_bytes: Option<u64>,
}

/// GPU discovered on the PCI bus.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GpuInfo {
    /// PCI bus/device/function address, such as `0000:03:00.0`. It is the
    /// stable identifier of the device and never changes between scans.
    pub pci_address: String,
    /// PCI vendor identifier, such as `0x1002`.
    pub vendor_id: String,
    /// PCI device identifier, such as `0x15bf`.
    pub device_id: String,
    /// PCI class code, such as `0x030000` for a display controller.
    pub class: String,
    /// Kernel driver bound to the device, when sysfs reports one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub driver: Option<String>,
}

/// Connection state of a display connector.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DisplayStatus {
    /// A display is attached.
    Connected,
    /// The connector is present but nothing is attached.
    Disconnected,
    /// The kernel does not publish a state.
    Unknown,
}

/// Display connector reported by the kernel DRM subsystem.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DisplayInfo {
    /// Connector name, such as `card0-DP-1`.
    pub connector: String,
    /// Whether something is attached.
    pub status: DisplayStatus,
    /// Whether the connector is enabled by the kernel, when published.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    /// Preferred or current mode, such as `1920x1080`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolution: Option<String>,
}

/// Subsystem that produced a diagnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Provider {
    /// Host operating system identity.
    System,
    /// CPU description.
    Cpu,
    /// Physical memory.
    Memory,
    /// PCI GPU enumeration.
    Gpu,
    /// Display connectors.
    Display,
    /// Vulkan enumeration, which this scan does not perform yet.
    Vulkan,
}

/// Severity of a diagnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticLevel {
    /// Explains what the report contains without degrading it.
    Info,
    /// A subsystem answered partially.
    Warning,
    /// A subsystem could not answer.
    Error,
}

/// One finding about one subsystem.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Diagnostic {
    /// Subsystem the finding belongs to.
    pub provider: Provider,
    /// How much the finding affects the report.
    pub level: DiagnosticLevel,
    /// Stable machine code, such as `source_unreadable`.
    pub code: String,
    /// Human explanation that names the path or the next step.
    pub message: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    fn minimal_report() -> HardwareReport {
        HardwareReport {
            schema_version: SCHEMA_VERSION.to_string(),
            scan_status: ScanStatus::Partial,
            system: SystemInfo {
                architecture: "x86_64".to_string(),
                kernel: None,
                distribution: None,
                libc: None,
            },
            cpu: None,
            gpus: Vec::new(),
            memory: None,
            displays: Vec::new(),
            diagnostics: Vec::new(),
        }
    }

    fn complete_report() -> HardwareReport {
        HardwareReport {
            schema_version: SCHEMA_VERSION.to_string(),
            scan_status: ScanStatus::Complete,
            system: SystemInfo {
                architecture: "x86_64".to_string(),
                kernel: Some("6.8.0-45-generic".to_string()),
                distribution: Some(Distribution {
                    id: "ubuntu".to_string(),
                    name: Some("Ubuntu".to_string()),
                    version_id: Some("24.04".to_string()),
                    pretty_name: Some("Ubuntu 24.04.1 LTS".to_string()),
                    id_like: vec!["debian".to_string()],
                    support: SupportLevel::Unknown,
                }),
                libc: Some(Libc {
                    family: LibcFamily::Glibc,
                    evidence: "/lib/x86_64-linux-gnu/libc.so.6".to_string(),
                }),
            },
            cpu: Some(CpuInfo {
                architecture: "x86_64".to_string(),
                vendor: Some("AuthenticAMD".to_string()),
                model: Some("AMD Ryzen 7 7840U".to_string()),
                logical_cores: 16,
                physical_cores: Some(8),
                sockets: Some(1),
                features: vec!["avx2".to_string(), "sse4_2".to_string()],
            }),
            gpus: vec![GpuInfo {
                pci_address: "0000:03:00.0".to_string(),
                vendor_id: "0x1002".to_string(),
                device_id: "0x15bf".to_string(),
                class: "0x030000".to_string(),
                driver: Some("amdgpu".to_string()),
            }],
            memory: Some(MemoryInfo {
                total_bytes: 33_403_977_728,
                available_bytes: Some(20_000_000_000),
                swap_total_bytes: Some(8_589_934_592),
            }),
            displays: vec![DisplayInfo {
                connector: "card0-eDP-1".to_string(),
                status: DisplayStatus::Connected,
                enabled: Some(true),
                resolution: Some("2880x1800".to_string()),
            }],
            diagnostics: vec![Diagnostic {
                provider: Provider::Vulkan,
                level: DiagnosticLevel::Info,
                code: "not_consulted".to_string(),
                message: "Vulkan enumeration is not part of this scan".to_string(),
            }],
        }
    }

    #[test]
    fn schema_version_is_declared_once_and_used_by_the_report() {
        let report = minimal_report();
        assert_eq!(report.schema_version, SCHEMA_VERSION);
        assert_eq!(SCHEMA_VERSION, "1.0");
    }

    #[test]
    fn report_serializes_the_documented_top_level_keys() {
        let value = serde_json::to_value(complete_report()).expect("report serializes");
        let object = value.as_object().expect("report is a JSON object");
        let mut keys: Vec<&str> = object.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            vec![
                "cpu",
                "diagnostics",
                "displays",
                "gpus",
                "memory",
                "scan_status",
                "schema_version",
                "system",
            ]
        );
    }

    #[test]
    fn absent_subsystems_are_omitted_instead_of_serialized_as_null() {
        let value = serde_json::to_value(minimal_report()).expect("report serializes");
        let object = value.as_object().expect("report is a JSON object");
        for absent in ["cpu", "memory"] {
            assert!(
                !object.contains_key(absent),
                "{absent} must be omitted when unknown, not serialized as null"
            );
        }
        assert_eq!(object["gpus"], json!([]));
        assert_eq!(object["displays"], json!([]));
        assert_eq!(object["diagnostics"], json!([]));
        assert!(
            !object["system"]
                .as_object()
                .expect("system is an object")
                .contains_key("kernel"),
            "an unknown kernel must be omitted"
        );
    }

    #[test]
    fn enums_serialize_as_snake_case_machine_names() {
        let value = serde_json::to_value(complete_report()).expect("report serializes");
        assert_eq!(value["scan_status"], json!("complete"));
        assert_eq!(value["system"]["distribution"]["support"], json!("unknown"));
        assert_eq!(value["system"]["libc"]["family"], json!("glibc"));
        assert_eq!(value["displays"][0]["status"], json!("connected"));
        assert_eq!(value["diagnostics"][0]["provider"], json!("vulkan"));
        assert_eq!(value["diagnostics"][0]["level"], json!("info"));
        assert_eq!(
            serde_json::to_value(ScanStatus::Partial).expect("status serializes"),
            json!("partial")
        );
        assert_eq!(
            serde_json::to_value(ScanStatus::Failed).expect("status serializes"),
            json!("failed")
        );
    }

    #[test]
    fn optional_empty_collections_stay_present_and_iterable() {
        let value = serde_json::to_value(minimal_report()).expect("report serializes");
        let gpus: &Vec<Value> = value["gpus"].as_array().expect("gpus is an array");
        assert!(gpus.is_empty());
    }
}
