//! Host operating system identity: distribution, kernel, libc and architecture.

use crate::model::{
    Diagnostic, DiagnosticLevel, Distribution, Libc, LibcFamily, Provider, SupportLevel, SystemInfo,
};
use crate::source::{HostSource, Subsystem, unreadable};

const OS_RELEASE: &str = "/etc/os-release";
const KERNEL_RELEASE: &str = "/proc/sys/kernel/osrelease";

/// Reads the host's operating system identity.
pub fn detect(source: &dyn HostSource) -> Subsystem<SystemInfo> {
    let architecture = std::env::consts::ARCH.to_string();
    let mut diagnostics = Vec::new();

    let distribution = match source.read(OS_RELEASE) {
        Ok(text) => {
            let parsed = parse_os_release(&text);
            if parsed.is_none() {
                diagnostics.push(Diagnostic {
                    provider: Provider::System,
                    level: DiagnosticLevel::Warning,
                    code: "distribution_unidentified".to_string(),
                    message: format!("{OS_RELEASE} declares no ID, so the distribution is unknown"),
                });
            }
            parsed
        }
        Err(error) => {
            diagnostics.push(unreadable(Provider::System, OS_RELEASE, &error));
            None
        }
    };

    let kernel = match source.read(KERNEL_RELEASE) {
        Ok(text) => {
            let release = text.trim();
            if release.is_empty() {
                diagnostics.push(Diagnostic {
                    provider: Provider::System,
                    level: DiagnosticLevel::Warning,
                    code: "kernel_unavailable".to_string(),
                    message: format!("{KERNEL_RELEASE} is empty, so the kernel release is unknown"),
                });
                None
            } else {
                Some(release.to_string())
            }
        }
        Err(error) => {
            diagnostics.push(Diagnostic {
                provider: Provider::System,
                level: DiagnosticLevel::Warning,
                code: "kernel_unavailable".to_string(),
                message: format!("could not read {KERNEL_RELEASE}: {error}"),
            });
            None
        }
    };

    let libc = detect_libc(source, &architecture);
    diagnostics.extend(libc.diagnostics);

    Subsystem::with_diagnostics(
        SystemInfo {
            architecture,
            kernel,
            distribution,
            libc: libc.value,
        },
        diagnostics,
    )
}

/// Parses `/etc/os-release` into a distribution identity.
///
/// Values may be quoted or bare; lines without a `=` separator are ignored,
/// including comments. `ID` is required: without it there is no distribution to
/// report.
pub fn parse_os_release(text: &str) -> Option<Distribution> {
    let mut id: Option<String> = None;
    let mut name: Option<String> = None;
    let mut version_id: Option<String> = None;
    let mut pretty_name: Option<String> = None;
    let mut id_like: Vec<String> = Vec::new();

    for line in text.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        if key.is_empty() {
            continue;
        }
        let value = unquote(value.trim());
        match key {
            "ID" if !value.is_empty() => id = Some(value),
            "NAME" if !value.is_empty() => name = Some(value),
            "VERSION_ID" if !value.is_empty() => version_id = Some(value),
            "PRETTY_NAME" if !value.is_empty() => pretty_name = Some(value),
            "ID_LIKE" => id_like = value.split_whitespace().map(str::to_string).collect(),
            _ => {}
        }
    }

    Some(Distribution {
        id: id?,
        name,
        version_id,
        pretty_name,
        id_like,
        support: SupportLevel::Unknown,
    })
}

/// Strips one layer of matching quotes from an `os-release` value.
fn unquote(value: &str) -> String {
    let bytes = value.as_bytes();
    if bytes.len() >= 2 {
        let first = bytes[0];
        let last = bytes[bytes.len() - 1];
        if (first == b'"' && last == b'"') || (first == b'\'' && last == b'\'') {
            return value[1..value.len() - 1].to_string();
        }
    }
    value.to_string()
}

/// Identifies the C library family by the loader or shared object it publishes.
///
/// This is a heuristic on purpose: the C library version is not part of the
/// kernel ABI, so the scan reports the evidence path next to the family and
/// never infers support from it.
pub fn detect_libc(source: &dyn HostSource, architecture: &str) -> Subsystem<Libc> {
    let musl = format!("/lib/ld-musl-{architecture}.so.1");
    if source.exists(&musl) {
        return Subsystem::complete(Libc {
            family: LibcFamily::Musl,
            evidence: musl,
        });
    }

    let candidates = [
        format!("/lib/{architecture}-linux-gnu/libc.so.6"),
        format!("/usr/lib/{architecture}-linux-gnu/libc.so.6"),
        "/lib64/libc.so.6".to_string(),
        "/lib/libc.so.6".to_string(),
        "/usr/lib/libc.so.6".to_string(),
    ];
    for candidate in candidates {
        if source.exists(&candidate) {
            return Subsystem::complete(Libc {
                family: LibcFamily::Glibc,
                evidence: candidate,
            });
        }
    }

    Subsystem::unavailable(vec![Diagnostic {
        provider: Provider::System,
        level: DiagnosticLevel::Info,
        code: "libc_unidentified".to_string(),
        message: format!(
            "no libc.so.6 nor ld-musl loader found for {architecture}; the C library stays unknown"
        ),
    }])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{LibcFamily, SupportLevel};
    use crate::source::MappingSource;

    const OS_RELEASE: &str = r#"NAME="Ubuntu"
VERSION="24.04.1 LTS (Noble Numbat)"
ID=ubuntu
ID_LIKE=debian
PRETTY_NAME="Ubuntu 24.04.1 LTS"
VERSION_ID="24.04"
HOME_URL="https://www.ubuntu.com/"
BUG_REPORT_URL="https://bugs.launchpad.net/ubuntu/"
"#;

    #[test]
    fn parses_the_documented_os_release_fields() {
        let distribution = parse_os_release(OS_RELEASE).expect("os-release declares ID");
        assert_eq!(distribution.id, "ubuntu");
        assert_eq!(distribution.name.as_deref(), Some("Ubuntu"));
        assert_eq!(distribution.version_id.as_deref(), Some("24.04"));
        assert_eq!(
            distribution.pretty_name.as_deref(),
            Some("Ubuntu 24.04.1 LTS")
        );
        assert_eq!(distribution.id_like, vec!["debian".to_string()]);
        assert_eq!(distribution.support, SupportLevel::Unknown);
    }

    #[test]
    fn quoted_and_unquoted_values_are_read_the_same_way() {
        let distribution =
            parse_os_release("ID=arch\nVERSION_ID=rolling\n").expect("os-release declares ID");
        assert_eq!(distribution.id, "arch");
        assert_eq!(distribution.version_id.as_deref(), Some("rolling"));
    }

    #[test]
    fn id_like_is_split_on_whitespace() {
        let distribution = parse_os_release("ID=fedora\nID_LIKE=\"rhel centos\"\n")
            .expect("os-release declares ID");
        assert_eq!(
            distribution.id_like,
            vec!["rhel".to_string(), "centos".to_string()]
        );
    }

    #[test]
    fn a_release_without_id_is_not_a_distribution() {
        assert!(parse_os_release("NAME=\"Anonymous\"\n").is_none());
        assert!(parse_os_release("").is_none());
    }

    #[test]
    fn lines_without_a_separator_are_ignored() {
        let distribution = parse_os_release("# comment\nID=alpine\nBROKEN LINE\n")
            .expect("os-release declares ID");
        assert_eq!(distribution.id, "alpine");
    }

    #[test]
    fn architecture_comes_from_the_binary_and_never_from_the_host() {
        let source = MappingSource::new().with_file("/etc/os-release", OS_RELEASE);
        let system = detect(&source);
        assert_eq!(
            system.value.expect("system is described").architecture,
            std::env::consts::ARCH
        );
    }

    #[test]
    fn kernel_release_is_trimmed_and_kept() {
        let source = MappingSource::new()
            .with_file("/etc/os-release", OS_RELEASE)
            .with_file("/proc/sys/kernel/osrelease", "6.8.0-45-generic\n");
        let system = detect(&source).value.expect("system is described");
        assert_eq!(system.kernel.as_deref(), Some("6.8.0-45-generic"));
    }

    #[test]
    fn a_missing_kernel_degrades_the_report_without_failing_it() {
        let source = MappingSource::new().with_file("/etc/os-release", OS_RELEASE);
        let system = detect(&source);
        assert!(system.value.is_some());
        assert!(!system.has_failures());
        assert_eq!(
            system.diagnostics.first().map(|d| d.level),
            Some(crate::model::DiagnosticLevel::Warning)
        );
        assert_eq!(system.diagnostics[0].code, "kernel_unavailable");
    }

    #[test]
    fn a_missing_os_release_is_an_error_that_names_the_path() {
        let source = MappingSource::new();
        let system = detect(&source);
        assert!(system.has_failures());
        assert!(
            system
                .diagnostics
                .iter()
                .any(|d| d.code == "source_unreadable" && d.message.contains("/etc/os-release"))
        );
    }

    #[test]
    fn musl_is_detected_from_its_dynamic_loader() {
        let source = MappingSource::new().with_present_path("/lib/ld-musl-x86_64.so.1");
        let libc = detect_libc(&source, "x86_64")
            .value
            .expect("libc is described");
        assert_eq!(libc.family, LibcFamily::Musl);
        assert!(libc.evidence.contains("ld-musl"));
    }

    #[test]
    fn glibc_is_detected_from_the_shared_object() {
        let source = MappingSource::new().with_present_path("/lib/x86_64-linux-gnu/libc.so.6");
        let libc = detect_libc(&source, "x86_64")
            .value
            .expect("libc is described");
        assert_eq!(libc.family, LibcFamily::Glibc);
        assert_eq!(libc.evidence, "/lib/x86_64-linux-gnu/libc.so.6");
    }

    #[test]
    fn an_unidentified_libc_is_reported_without_failing_the_scan() {
        let source = MappingSource::new();
        let libc = detect_libc(&source, "x86_64");
        assert!(libc.value.is_none());
        assert!(!libc.has_failures());
        assert_eq!(libc.diagnostics[0].code, "libc_unidentified");
    }

    #[test]
    fn system_provider_reports_every_subsystem_it_could_read() {
        let source = MappingSource::new()
            .with_file("/etc/os-release", OS_RELEASE)
            .with_file("/proc/sys/kernel/osrelease", "6.8.0-45-generic\n")
            .with_present_path("/lib/x86_64-linux-gnu/libc.so.6");
        let system = detect(&source);
        let value = system.value.expect("system is described");
        assert_eq!(value.distribution.expect("distribution").id, "ubuntu");
        assert_eq!(
            value.libc.expect("libc").family,
            LibcFamily::Glibc,
            "libc detection must not depend on the host architecture name"
        );
        assert!(system.diagnostics.is_empty());
    }

    #[test]
    fn provider_is_always_system_for_its_own_findings() {
        let source = MappingSource::new();
        for diagnostic in detect(&source).diagnostics {
            assert_eq!(diagnostic.provider, crate::model::Provider::System);
        }
        for diagnostic in detect_libc(&source, "x86_64").diagnostics {
            assert_eq!(diagnostic.provider, crate::model::Provider::System);
        }
    }
}
