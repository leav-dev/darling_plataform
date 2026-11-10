//! Hardware Intelligence Engine (DHI).
//!
//! Responsibility: discover CPU, memory, distribution, kernel, libc, PCI GPUs,
//! Vulkan capabilities and displays, and publish them as a versioned report
//! with per-subsystem diagnostics.
//!
//! Not responsible for: installing drivers, changing host configuration, or
//! choosing a GPU/backend for a launch. Detection describes; the runtime
//! decides.
//!
//! Discovery must stay partial and fault tolerant: an unavailable subsystem
//! degrades the report instead of invalidating it, and no scan requires root.

pub mod cpu;
pub mod display;
pub mod gpu;
pub mod memory;
pub mod model;
pub mod scan;
pub mod source;
pub mod system;

pub use model::{
    CpuInfo, Diagnostic, DiagnosticLevel, DisplayInfo, DisplayStatus, Distribution, GpuInfo,
    HardwareReport, Libc, LibcFamily, MemoryInfo, Provider, SCHEMA_VERSION, ScanStatus,
    SupportLevel, SystemInfo,
};
pub use scan::{scan, scan_current_host};
pub use source::{HostSource, LinuxHost, MappingSource};

#[cfg(test)]
mod tests {
    #[test]
    fn declares_its_responsibility() {
        assert!(!env!("CARGO_PKG_DESCRIPTION").trim().is_empty());
    }
}
