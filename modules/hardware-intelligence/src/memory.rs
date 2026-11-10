//! Physical memory from `/proc/meminfo`.

use crate::model::{Diagnostic, DiagnosticLevel, MemoryInfo, Provider};
use crate::source::{HostSource, Subsystem, unreadable};

const MEMINFO: &str = "/proc/meminfo";

/// Bytes per kibibyte, the unit every `meminfo` value uses.
const KIB: u64 = 1024;

/// Reads the host's physical memory description.
pub fn detect(source: &dyn HostSource) -> Subsystem<MemoryInfo> {
    match source.read(MEMINFO) {
        Ok(text) => parse_meminfo(&text),
        Err(error) => Subsystem::unavailable(vec![unreadable(Provider::Memory, MEMINFO, &error)]),
    }
}

/// Parses the kernel's `meminfo` text.
///
/// Values are matched by name, never by position, and always reported in bytes.
/// `MemTotal` is the only required field: without it the host's memory cannot be
/// described.
pub fn parse_meminfo(text: &str) -> Subsystem<MemoryInfo> {
    let mut total: Option<u64> = None;
    let mut available: Option<u64> = None;
    let mut swap_total: Option<u64> = None;

    for line in text.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let Some(kibibytes) = value
            .split_whitespace()
            .next()
            .and_then(|v| v.parse::<u64>().ok())
        else {
            continue;
        };
        match key.trim() {
            "MemTotal" => total = Some(kibibytes.saturating_mul(KIB)),
            "MemAvailable" => available = Some(kibibytes.saturating_mul(KIB)),
            "SwapTotal" => swap_total = Some(kibibytes.saturating_mul(KIB)),
            _ => {}
        }
    }

    let Some(total_bytes) = total else {
        return Subsystem::unavailable(vec![Diagnostic {
            provider: Provider::Memory,
            level: DiagnosticLevel::Error,
            code: "mem_total_missing".to_string(),
            message: format!("{MEMINFO} has no MemTotal, so system memory cannot be described"),
        }]);
    };

    Subsystem::complete(MemoryInfo {
        total_bytes,
        available_bytes: available,
        swap_total_bytes: swap_total,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::MappingSource;

    const MEMINFO: &str = r#"MemTotal:       32608376 kB
MemFree:         1234567 kB
MemAvailable:   20480000 kB
Buffers:          123456 kB
Cached:          9876543 kB
SwapTotal:       8388604 kB
SwapFree:        8388604 kB
"#;

    #[test]
    fn mem_total_is_converted_from_kibibytes_to_bytes() {
        let memory = parse_meminfo(MEMINFO).value.expect("meminfo is described");
        assert_eq!(memory.total_bytes, 32_608_376 * 1024);
    }

    #[test]
    fn available_memory_and_swap_are_reported_when_present() {
        let memory = parse_meminfo(MEMINFO).value.expect("meminfo is described");
        assert_eq!(memory.available_bytes, Some(20_480_000 * 1024));
        assert_eq!(memory.swap_total_bytes, Some(8_388_604 * 1024));
    }

    #[test]
    fn swap_and_availability_are_optional() {
        let memory = parse_meminfo("MemTotal: 1024 kB\n")
            .value
            .expect("meminfo is described");
        assert_eq!(memory.total_bytes, 1_048_576);
        assert_eq!(memory.available_bytes, None);
        assert_eq!(memory.swap_total_bytes, None);
    }

    #[test]
    fn fields_are_matched_by_name_not_by_position() {
        let memory =
            parse_meminfo("SwapTotal: 2048 kB\nMemTotal: 4096 kB\nMemAvailable: 1024 kB\n")
                .value
                .expect("meminfo is described");
        assert_eq!(memory.total_bytes, 4_194_304);
        assert_eq!(memory.available_bytes, Some(1_048_576));
        assert_eq!(memory.swap_total_bytes, Some(2_097_152));
    }

    #[test]
    fn a_missing_mem_total_is_an_error_that_names_the_field() {
        let memory = parse_meminfo("MemFree: 1024 kB\n");
        assert!(memory.value.is_none());
        assert!(memory.has_failures());
        assert_eq!(memory.diagnostics[0].code, "mem_total_missing");
        assert!(memory.diagnostics[0].message.contains("MemTotal"));
    }

    #[test]
    fn detect_reads_meminfo_through_the_injected_source() {
        let source = MappingSource::new().with_file("/proc/meminfo", MEMINFO);
        let memory = detect(&source).value.expect("memory is described");
        assert_eq!(memory.total_bytes, 32_608_376 * 1024);
    }

    #[test]
    fn an_unreadable_meminfo_is_an_error_for_the_memory_provider() {
        let source = MappingSource::new();
        let memory = detect(&source);
        assert!(memory.value.is_none());
        assert!(memory.has_failures());
        assert_eq!(
            memory.diagnostics[0].provider,
            crate::model::Provider::Memory
        );
        assert!(memory.diagnostics[0].message.contains("/proc/meminfo"));
    }
}
