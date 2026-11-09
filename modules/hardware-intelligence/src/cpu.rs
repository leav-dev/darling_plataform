//! CPU description from `/proc/cpuinfo`.

use std::collections::BTreeSet;

use crate::model::{CpuInfo, Diagnostic, DiagnosticLevel, Provider};
use crate::source::{HostSource, Subsystem, unreadable};

const CPUINFO: &str = "/proc/cpuinfo";

/// Reads the host's CPU description.
pub fn detect(source: &dyn HostSource) -> Subsystem<CpuInfo> {
    match source.read(CPUINFO) {
        Ok(text) => parse_cpuinfo(&text, std::env::consts::ARCH),
        Err(error) => Subsystem::unavailable(vec![unreadable(Provider::Cpu, CPUINFO, &error)]),
    }
}

/// Parses the kernel's `cpuinfo` text.
///
/// The format is a sequence of processor stanzas with `key\t: value` lines. The
/// kernel publishes the same facts under different names depending on the
/// architecture, so absent fields degrade the description instead of failing it.
pub fn parse_cpuinfo(text: &str, architecture: &str) -> Subsystem<CpuInfo> {
    let mut logical_cores: u32 = 0;
    let mut vendor: Option<String> = None;
    let mut model: Option<String> = None;
    let mut sockets: BTreeSet<String> = BTreeSet::new();
    let mut cores_per_socket: Option<u32> = None;
    let mut features: BTreeSet<String> = BTreeSet::new();

    for line in text.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim();
        if value.is_empty() {
            continue;
        }
        match key {
            "processor" => logical_cores = logical_cores.saturating_add(1),
            "vendor_id" => {
                vendor.get_or_insert_with(|| value.to_string());
            }
            "model name" => {
                model.get_or_insert_with(|| value.to_string());
            }
            "physical id" => {
                sockets.insert(value.to_string());
            }
            "cpu cores" => {
                if let Ok(cores) = value.parse::<u32>() {
                    cores_per_socket =
                        Some(cores_per_socket.map_or(cores, |current| current.max(cores)));
                }
            }
            "flags" | "Features" => {
                features.extend(value.split_whitespace().map(str::to_string));
            }
            _ => {}
        }
    }

    if logical_cores == 0 {
        return Subsystem::unavailable(vec![Diagnostic {
            provider: Provider::Cpu,
            level: DiagnosticLevel::Error,
            code: "cpuinfo_empty".to_string(),
            message: format!("{CPUINFO} lists no processor, so the CPU cannot be described"),
        }]);
    }

    let sockets_count = if sockets.is_empty() {
        None
    } else {
        Some(sockets.len() as u32)
    };
    let physical_cores = match (sockets_count, cores_per_socket) {
        (Some(sockets), Some(cores)) => Some(sockets.saturating_mul(cores)),
        _ => None,
    };

    let mut diagnostics = Vec::new();
    if model.is_none() {
        diagnostics.push(Diagnostic {
            provider: Provider::Cpu,
            level: DiagnosticLevel::Info,
            code: "cpu_model_unavailable".to_string(),
            message: format!(
                "the kernel publishes no model name in {CPUINFO}; the report identifies this CPU as {architecture}"
            ),
        });
    }

    let cpu = CpuInfo {
        architecture: architecture.to_string(),
        vendor,
        model,
        logical_cores,
        physical_cores,
        sockets: sockets_count,
        features: features.into_iter().collect(),
    };
    Subsystem::with_diagnostics(cpu, diagnostics)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::DiagnosticLevel;
    use crate::source::MappingSource;

    const CPUINFO_TWO_SOCKETS: &str = r#"processor	: 0
vendor_id	: AuthenticAMD
cpu family	: 25
model		: 116
model name	: AMD Ryzen 7 7840U w/ Radeon 780M Graphics
physical id	: 0
siblings	: 2
cpu cores	: 2
flags		: fpu vme de pse msr avx2 sse4_2

processor	: 1
vendor_id	: AuthenticAMD
model name	: AMD Ryzen 7 7840U w/ Radeon 780M Graphics
physical id	: 0
siblings	: 2
cpu cores	: 2
flags		: fpu avx2 avx512f sse4_2

processor	: 2
vendor_id	: AuthenticAMD
model name	: AMD Ryzen 7 7840U w/ Radeon 780M Graphics
physical id	: 1
siblings	: 2
cpu cores	: 2
flags		: fpu avx2

processor	: 3
vendor_id	: AuthenticAMD
model name	: AMD Ryzen 7 7840U w/ Radeon 780M Graphics
physical id	: 1
siblings	: 2
cpu cores	: 2
flags		: fpu avx2
"#;

    const CPUINFO_AARCH64: &str = r#"processor	: 0
BogoMIPS	: 48.00
Features	: fp asimd evtstrm aes pmull sha1 sha2 crc32
CPU implementer	: 0x41
CPU architecture: 8
CPU variant	: 0x0
CPU part	: 0xd0c
CPU revision	: 1
"#;

    #[test]
    fn reads_vendor_and_model_from_cpuinfo() {
        let cpu = parse_cpuinfo(CPUINFO_TWO_SOCKETS, "x86_64")
            .value
            .expect("cpuinfo is described");
        assert_eq!(cpu.vendor.as_deref(), Some("AuthenticAMD"));
        assert_eq!(
            cpu.model.as_deref(),
            Some("AMD Ryzen 7 7840U w/ Radeon 780M Graphics")
        );
        assert_eq!(cpu.architecture, "x86_64");
    }

    #[test]
    fn counts_one_logical_processor_per_stanza() {
        let cpu = parse_cpuinfo(CPUINFO_TWO_SOCKETS, "x86_64")
            .value
            .expect("cpuinfo is described");
        assert_eq!(cpu.logical_cores, 4);
    }

    #[test]
    fn derives_physical_cores_and_sockets_from_the_topology() {
        let cpu = parse_cpuinfo(CPUINFO_TWO_SOCKETS, "x86_64")
            .value
            .expect("cpuinfo is described");
        assert_eq!(cpu.sockets, Some(2));
        assert_eq!(cpu.physical_cores, Some(4));
    }

    #[test]
    fn features_are_the_sorted_union_of_every_processor() {
        let cpu = parse_cpuinfo(CPUINFO_TWO_SOCKETS, "x86_64")
            .value
            .expect("cpuinfo is described");
        assert_eq!(
            cpu.features,
            vec![
                "avx2".to_string(),
                "avx512f".to_string(),
                "de".to_string(),
                "fpu".to_string(),
                "msr".to_string(),
                "pse".to_string(),
                "sse4_2".to_string(),
                "vme".to_string(),
            ]
        );
    }

    #[test]
    fn aarch64_reports_its_features_and_flags_the_missing_model() {
        let cpu = parse_cpuinfo(CPUINFO_AARCH64, "aarch64");
        assert_eq!(cpu.diagnostics[0].code, "cpu_model_unavailable");
        assert_eq!(cpu.diagnostics[0].level, DiagnosticLevel::Info);
        assert!(!cpu.has_failures());
        let value = cpu.value.expect("cpuinfo is described");
        assert_eq!(value.vendor, None);
        assert_eq!(value.model, None);
        assert_eq!(value.logical_cores, 1);
        assert_eq!(value.sockets, None, "aarch64 publishes no physical id");
        assert_eq!(value.physical_cores, None);
        assert_eq!(
            value.features,
            vec![
                "aes".to_string(),
                "asimd".to_string(),
                "crc32".to_string(),
                "evtstrm".to_string(),
                "fp".to_string(),
                "pmull".to_string(),
                "sha1".to_string(),
                "sha2".to_string(),
            ]
        );
    }

    #[test]
    fn empty_cpuinfo_is_unavailable_and_names_the_path() {
        let cpu = parse_cpuinfo("", "x86_64");
        assert!(cpu.value.is_none());
        assert!(cpu.has_failures());
        assert_eq!(cpu.diagnostics[0].code, "cpuinfo_empty");
    }

    #[test]
    fn detect_reads_cpuinfo_through_the_injected_source() {
        let source = MappingSource::new().with_file("/proc/cpuinfo", CPUINFO_TWO_SOCKETS);
        let cpu = detect(&source).value.expect("cpu is described");
        assert_eq!(cpu.logical_cores, 4);
    }

    #[test]
    fn an_unreadable_cpuinfo_is_an_error_for_the_cpu_provider() {
        let source = MappingSource::new();
        let cpu = detect(&source);
        assert!(cpu.value.is_none());
        assert!(cpu.has_failures());
        assert_eq!(cpu.diagnostics[0].provider, crate::model::Provider::Cpu);
        assert!(cpu.diagnostics[0].message.contains("/proc/cpuinfo"));
    }
}
