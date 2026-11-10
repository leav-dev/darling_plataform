//! PCI GPU enumeration from sysfs.
//!
//! Devices are discovered on the PCI bus and identified by their bus/device/
//! function address, which is stable across scans and reboots. The provider only
//! describes what the kernel publishes: it never loads a driver or probes a GPU
//! itself, so the scan needs no privileges.

use crate::model::{Diagnostic, DiagnosticLevel, GpuInfo, Provider};
use crate::source::{HostSource, Subsystem, unreadable};

const PCI_DEVICES: &str = "/sys/bus/pci/devices";

/// First two digits of the PCI base class code of a display controller.
const DISPLAY_CLASS_PREFIX: &str = "0x03";

/// Enumerates the display controllers on the host's PCI bus.
pub fn detect(source: &dyn HostSource) -> Subsystem<Vec<GpuInfo>> {
    let entries = match source.read_dir(PCI_DEVICES) {
        Ok(entries) => entries,
        Err(error) => {
            return Subsystem::unavailable(vec![unreadable(Provider::Gpu, PCI_DEVICES, &error)]);
        }
    };

    let mut gpus = Vec::new();
    let mut diagnostics = Vec::new();

    for pci_address in entries {
        let directory = format!("{PCI_DEVICES}/{pci_address}");
        let class = match read_trimmed(source, &format!("{directory}/class")) {
            Some(class) => class,
            None => {
                diagnostics.push(Diagnostic {
                    provider: Provider::Gpu,
                    level: DiagnosticLevel::Warning,
                    code: "device_class_unreadable".to_string(),
                    message: format!(
                        "could not read {directory}/class, so {pci_address} cannot be classified as a display controller"
                    ),
                });
                continue;
            }
        };
        if !class.starts_with(DISPLAY_CLASS_PREFIX) {
            continue;
        }

        let (Some(vendor_id), Some(device_id)) = (
            read_trimmed(source, &format!("{directory}/vendor")),
            read_trimmed(source, &format!("{directory}/device")),
        ) else {
            diagnostics.push(Diagnostic {
                provider: Provider::Gpu,
                level: DiagnosticLevel::Warning,
                code: "device_identity_incomplete".to_string(),
                message: format!(
                    "{pci_address} is a display controller but publishes no vendor/device pair, so it cannot be identified"
                ),
            });
            continue;
        };

        gpus.push(GpuInfo {
            driver: read_driver_name(source, &directory),
            pci_address,
            vendor_id,
            device_id,
            class,
        });
    }

    gpus.sort_by(|left, right| left.pci_address.cmp(&right.pci_address));
    Subsystem::with_diagnostics(gpus, diagnostics)
}

/// Reads a sysfs attribute and trims the trailing newline the kernel writes.
fn read_trimmed(source: &dyn HostSource, path: &str) -> Option<String> {
    let value = source.read(path).ok()?;
    let value = value.trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

/// Resolves the bound kernel driver from the device's `driver` link.
///
/// An unbound device is normal, not a problem: the module may simply not be
/// loaded, so the absence is reported as `None` instead of a diagnostic.
fn read_driver_name(source: &dyn HostSource, directory: &str) -> Option<String> {
    let target = source.read_link(&format!("{directory}/driver")).ok()?;
    target
        .rsplit('/')
        .find(|segment| !segment.is_empty())
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::DiagnosticLevel;
    use crate::source::MappingSource;

    const PCI: &str = "/sys/bus/pci/devices";

    fn host_with_two_devices() -> MappingSource {
        MappingSource::new()
            .with_dir(PCI, ["0000:03:00.0", "0000:05:00.0", "0000:06:00.0"])
            .with_file("/sys/bus/pci/devices/0000:03:00.0/class", "0x030000\n")
            .with_file("/sys/bus/pci/devices/0000:03:00.0/vendor", "0x1002\n")
            .with_file("/sys/bus/pci/devices/0000:03:00.0/device", "0x15bf\n")
            .with_link(
                "/sys/bus/pci/devices/0000:03:00.0/driver",
                "../../../bus/pci/drivers/amdgpu",
            )
            .with_file("/sys/bus/pci/devices/0000:05:00.0/class", "0x020000\n")
            .with_file("/sys/bus/pci/devices/0000:05:00.0/vendor", "0x8086\n")
            .with_file("/sys/bus/pci/devices/0000:05:00.0/device", "0x15f3\n")
            .with_file("/sys/bus/pci/devices/0000:06:00.0/vendor", "0x10de\n")
            .with_file("/sys/bus/pci/devices/0000:06:00.0/device", "0x28e0\n")
    }

    #[test]
    fn enumerates_only_display_controllers() {
        let gpus = detect(&host_with_two_devices())
            .value
            .expect("the PCI bus is readable");
        assert_eq!(gpus.len(), 1, "a network controller is not a GPU");
        assert_eq!(gpus[0].class, "0x030000");
    }

    #[test]
    fn keeps_the_stable_pci_address_as_the_identifier() {
        let gpus = detect(&host_with_two_devices())
            .value
            .expect("the PCI bus is readable");
        assert_eq!(gpus[0].pci_address, "0000:03:00.0");
    }

    #[test]
    fn reads_vendor_and_device_identifiers() {
        let gpus = detect(&host_with_two_devices())
            .value
            .expect("the PCI bus is readable");
        assert_eq!(gpus[0].vendor_id, "0x1002");
        assert_eq!(gpus[0].device_id, "0x15bf");
    }

    #[test]
    fn reports_the_bound_kernel_driver() {
        let gpus = detect(&host_with_two_devices())
            .value
            .expect("the PCI bus is readable");
        assert_eq!(gpus[0].driver.as_deref(), Some("amdgpu"));
    }

    #[test]
    fn a_device_without_a_readable_class_is_skipped_with_a_warning() {
        let gpus = detect(&host_with_two_devices());
        assert!(!gpus.has_failures());
        let warning = gpus
            .diagnostics
            .iter()
            .find(|d| d.code == "device_class_unreadable")
            .expect("the unclassifiable device is reported");
        assert_eq!(warning.level, DiagnosticLevel::Warning);
        assert!(warning.message.contains("0000:06:00.0"));
        let value = gpus.value.expect("the PCI bus is readable");
        assert_eq!(
            value.len(),
            1,
            "0000:06:00.0 has no class and cannot be classified"
        );
    }

    #[test]
    fn a_host_without_display_controllers_reports_an_empty_list() {
        let source = MappingSource::new().with_dir(PCI, ["0000:00:00.0"]);
        let gpus = detect(&source);
        assert!(!gpus.has_failures());
        let value = gpus.value.expect("the PCI bus is readable");
        assert!(value.is_empty());
    }

    #[test]
    fn devices_are_ordered_by_pci_address() {
        let source = MappingSource::new()
            .with_dir(PCI, ["0000:0b:00.0", "0000:03:00.0"])
            .with_file("/sys/bus/pci/devices/0000:0b:00.0/class", "0x030200\n")
            .with_file("/sys/bus/pci/devices/0000:0b:00.0/vendor", "0x1002\n")
            .with_file("/sys/bus/pci/devices/0000:0b:00.0/device", "0x164e\n")
            .with_file("/sys/bus/pci/devices/0000:03:00.0/class", "0x030000\n")
            .with_file("/sys/bus/pci/devices/0000:03:00.0/vendor", "0x1002\n")
            .with_file("/sys/bus/pci/devices/0000:03:00.0/device", "0x15bf\n");
        let gpus = detect(&source).value.expect("the PCI bus is readable");
        let addresses: Vec<&str> = gpus.iter().map(|gpu| gpu.pci_address.as_str()).collect();
        assert_eq!(addresses, vec!["0000:03:00.0", "0000:0b:00.0"]);
    }

    #[test]
    fn an_unreadable_pci_bus_is_an_error_for_the_gpu_provider() {
        let gpus = detect(&MappingSource::new());
        assert!(gpus.value.is_none());
        assert_eq!(gpus.diagnostics[0].provider, crate::model::Provider::Gpu);
        assert!(gpus.diagnostics[0].message.contains(PCI));
        assert!(gpus.has_failures());
    }
}
