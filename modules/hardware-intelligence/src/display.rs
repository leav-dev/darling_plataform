//! Display connectors from the kernel DRM tree.
//!
//! The scan reads `/sys/class/drm`, where the kernel publishes one node per
//! connector next to the cards and render nodes. Connection state is what the
//! kernel measured; resolution is the first mode the connector advertises, which
//! is the preferred mode and not necessarily the one a compositor chose.

use crate::model::{Diagnostic, DiagnosticLevel, DisplayInfo, DisplayStatus, Provider};
use crate::source::{HostSource, Subsystem, unreadable};

const DRM: &str = "/sys/class/drm";

/// Enumerates the display connectors the kernel exposes.
pub fn detect(source: &dyn HostSource) -> Subsystem<Vec<DisplayInfo>> {
    let entries = match source.read_dir(DRM) {
        Ok(entries) => entries,
        Err(error) => {
            return Subsystem::unavailable(vec![unreadable(Provider::Display, DRM, &error)]);
        }
    };

    let mut displays = Vec::new();
    let mut diagnostics = Vec::new();

    for connector in entries {
        if !is_connector(&connector) {
            continue;
        }
        let directory = format!("{DRM}/{connector}");
        let status = match source.read(&format!("{directory}/status")) {
            Ok(text) => parse_status(text.trim()),
            Err(error) => {
                diagnostics.push(Diagnostic {
                    provider: Provider::Display,
                    level: DiagnosticLevel::Warning,
                    code: "status_unreadable".to_string(),
                    message: format!("could not read {directory}/status: {error}"),
                });
                DisplayStatus::Unknown
            }
        };
        let enabled = source
            .read(&format!("{directory}/enabled"))
            .ok()
            .and_then(|text| match text.trim() {
                "enabled" => Some(true),
                "disabled" => Some(false),
                _ => None,
            });
        let resolution = source
            .read(&format!("{directory}/modes"))
            .ok()
            .and_then(|text| {
                text.lines()
                    .map(str::trim)
                    .find(|line| !line.is_empty())
                    .map(str::to_string)
            });

        displays.push(DisplayInfo {
            connector,
            status,
            enabled,
            resolution,
        });
    }

    if displays.is_empty() {
        diagnostics.push(Diagnostic {
            provider: Provider::Display,
            level: DiagnosticLevel::Info,
            code: "no_drm_connectors".to_string(),
            message: format!("{DRM} publishes no connector; the host may be headless"),
        });
    }

    Subsystem::with_diagnostics(displays, diagnostics)
}

/// A DRM connector is a `card<N>-<TYPE>` node; cards and render nodes are not.
fn is_connector(entry: &str) -> bool {
    entry.starts_with("card") && entry.contains('-')
}

/// Maps the kernel's connector state to the report's vocabulary.
fn parse_status(value: &str) -> DisplayStatus {
    match value {
        "connected" => DisplayStatus::Connected,
        "disconnected" => DisplayStatus::Disconnected,
        _ => DisplayStatus::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::DisplayStatus;
    use crate::source::MappingSource;

    const DRM: &str = "/sys/class/drm";

    fn host_with_displays() -> MappingSource {
        MappingSource::new()
            .with_dir(
                DRM,
                [
                    "card0",
                    "card0-DP-1",
                    "card0-eDP-1",
                    "renderD128",
                    "version",
                ],
            )
            .with_file("/sys/class/drm/card0-eDP-1/status", "connected\n")
            .with_file("/sys/class/drm/card0-eDP-1/enabled", "enabled\n")
            .with_file("/sys/class/drm/card0-eDP-1/modes", "2880x1800\n1920x1200\n")
            .with_file("/sys/class/drm/card0-DP-1/status", "disconnected\n")
            .with_file("/sys/class/drm/card0-DP-1/enabled", "disabled\n")
    }

    #[test]
    fn enumerates_connectors_and_ignores_cards_and_render_nodes() {
        let displays = detect(&host_with_displays())
            .value
            .expect("the DRM tree is readable");
        let connectors: Vec<&str> = displays
            .iter()
            .map(|display| display.connector.as_str())
            .collect();
        assert_eq!(connectors, vec!["card0-DP-1", "card0-eDP-1"]);
    }

    #[test]
    fn reads_connection_state_enabled_flag_and_first_mode() {
        let displays = detect(&host_with_displays())
            .value
            .expect("the DRM tree is readable");
        let internal = displays
            .iter()
            .find(|display| display.connector == "card0-eDP-1")
            .expect("the internal panel is listed");
        assert_eq!(internal.status, DisplayStatus::Connected);
        assert_eq!(internal.enabled, Some(true));
        assert_eq!(internal.resolution.as_deref(), Some("2880x1800"));
    }

    #[test]
    fn a_disconnected_connector_has_no_resolution() {
        let displays = detect(&host_with_displays())
            .value
            .expect("the DRM tree is readable");
        let external = displays
            .iter()
            .find(|display| display.connector == "card0-DP-1")
            .expect("the external connector is listed");
        assert_eq!(external.status, DisplayStatus::Disconnected);
        assert_eq!(external.enabled, Some(false));
        assert_eq!(external.resolution, None);
    }

    #[test]
    fn an_unreadable_status_is_unknown_instead_of_failing_the_provider() {
        let source = MappingSource::new()
            .with_dir(DRM, ["card0-HDMI-A-1"])
            .with_file("/sys/class/drm/card0-HDMI-A-1/enabled", "enabled\n");
        let displays = detect(&source);
        assert!(!displays.has_failures());
        assert_eq!(displays.diagnostics[0].code, "status_unreadable");
        let value = displays.value.expect("the DRM tree is readable");
        assert_eq!(value[0].status, DisplayStatus::Unknown);
    }

    #[test]
    fn a_headless_host_reports_no_displays_and_no_failure() {
        let source = MappingSource::new().with_dir(DRM, ["version"]);
        let displays = detect(&source);
        assert!(!displays.has_failures());
        assert_eq!(displays.diagnostics[0].code, "no_drm_connectors");
        assert!(displays.value.expect("the DRM tree is readable").is_empty());
    }

    #[test]
    fn an_unreadable_drm_tree_is_an_error_for_the_display_provider() {
        let displays = detect(&MappingSource::new());
        assert!(displays.value.is_none());
        assert_eq!(
            displays.diagnostics[0].provider,
            crate::model::Provider::Display
        );
        assert!(displays.diagnostics[0].message.contains(DRM));
        assert!(displays.has_failures());
    }
}
