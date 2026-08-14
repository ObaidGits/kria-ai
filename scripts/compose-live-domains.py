#!/usr/bin/env python3
"""Compose the twelve remaining OS-control domains into the live aggregate.

Adds, for each domain: a struct field, a `compose_with` initializer, the
`HostOsControl` trait method, and a `composed_domains()` entry.

Written as a script rather than a chain of edits because all four edits must land
together — a field without an initializer will not compile, and a half-applied
composition is worse than none.
"""
import pathlib
import re
import sys

LIVE = pathlib.Path("crates/kria-core/src/os_control/live.rs")

# (field, port type, trait method, composition expression, domain label)
DOMAINS = [
    (
        "search_control",
        "crate::os_control::search::SearchControlPort",
        "search_control",
        """session_ok
                .then(crate::os_control::linux::providers::tracker_search::LiveSearch::discover)
                .flatten()
                .map(|transport| {
                    Arc::new(crate::os_control::search::SearchControl::new(transport))
                        as Arc<dyn crate::os_control::search::SearchControlPort>
                })""",
        "search",
    ),
    (
        "health",
        "crate::os_control::health::HealthControlPort",
        "health",
        """Some(Arc::new(crate::os_control::health::SystemHealthControl::new(
                crate::os_control::linux::providers::system_health::LiveHealth::discover(),
            )) as Arc<dyn crate::os_control::health::HealthControlPort>)""",
        "health",
    ),
    (
        "backup_scan",
        "crate::os_control::backup::BackupScanControlPort",
        "backup_scan",
        """crate::os_control::linux::providers::backup_scan::LiveBackupScan::discover().map(
                |transport| {
                    Arc::new(crate::os_control::backup::BackupScanControl::new(transport))
                        as Arc<dyn crate::os_control::backup::BackupScanControlPort>
                },
            )""",
        "backup_scan",
    ),
    (
        "firmware",
        "crate::os_control::hardware::FirmwareAwarenessPort",
        "firmware",
        """crate::os_control::linux::providers::firmware_sensors::LiveFirmware::discover().map(
                |provider| {
                    Arc::new(provider) as Arc<dyn crate::os_control::hardware::FirmwareAwarenessPort>
                },
            )""",
        "firmware",
    ),
    (
        "hardware",
        "crate::os_control::hardware::HardwareControlPort",
        "hardware",
        """crate::os_control::linux::providers::firmware_sensors::LiveHardwareSensors::discover()
                .map(|provider| {
                    Arc::new(provider) as Arc<dyn crate::os_control::hardware::HardwareControlPort>
                })""",
        "hardware",
    ),
    (
        "print_control",
        "crate::os_control::print::PrintControlPort",
        "print_control",
        """crate::os_control::linux::providers::cups_print::LivePrint::discover().map(
                |transport| {
                    Arc::new(crate::os_control::print::PrintControl::new(transport))
                        as Arc<dyn crate::os_control::print::PrintControlPort>
                },
            )""",
        "print",
    ),
    (
        "privacy",
        "crate::os_control::privacy::PrivacyControlPort",
        "privacy",
        """session_ok
                .then(
                    crate::os_control::linux::providers::privacy_firewall::LivePrivacy::discover,
                )
                .flatten()
                .map(|transport| {
                    Arc::new(crate::os_control::privacy::PrivacyControl::new(transport))
                        as Arc<dyn crate::os_control::privacy::PrivacyControlPort>
                })""",
        "privacy",
    ),
    (
        "firewall",
        "crate::os_control::firewall::FirewallControlPort",
        "firewall",
        """crate::os_control::linux::providers::privacy_firewall::LiveFirewall::discover().map(
                |transport| {
                    Arc::new(crate::os_control::firewall::FirewallControl::new(transport))
                        as Arc<dyn crate::os_control::firewall::FirewallControlPort>
                },
            )""",
        "firewall",
    ),
    (
        "display_configuration",
        "crate::os_control::display::configuration::DisplayConfigControlPort",
        "display_configuration",
        """session_ok
                .then(
                    crate::os_control::linux::providers::display_config::LiveDisplayConfig::discover,
                )
                .flatten()
                .map(|transport| {
                    Arc::new(
                        crate::os_control::display::configuration::DisplayConfigControl::new(
                            transport,
                        ),
                    )
                        as Arc<
                            dyn crate::os_control::display::configuration::DisplayConfigControlPort,
                        >
                })""",
        "display_configuration",
    ),
    (
        "desktop_association",
        "crate::os_control::applications::DesktopAssociationControlPort",
        "desktop_association",
        """session_ok.then(|| {
                Arc::new(crate::os_control::applications::DesktopAssociationControl::new(
                    crate::os_control::applications::RealDesktopAssociationTransport::new(),
                ))
                    as Arc<dyn crate::os_control::applications::DesktopAssociationControlPort>
            })""",
        "desktop_association",
    ),
    (
        "automation",
        "crate::os_control::automation::AutomationControlPort",
        "automation",
        """Some(Arc::new(crate::os_control::automation::AutomationControl::new(
                crate::os_control::linux::providers::automation::LiveAutomation::new(token),
            )) as Arc<dyn crate::os_control::automation::AutomationControlPort>)""",
        "automation",
    ),
    (
        "charge_thresholds",
        "crate::os_control::power::charge::ChargeThresholdControlPort",
        "charge_thresholds",
        """None""",
        "charge_thresholds",
    ),
]


def main() -> int:
    text = LIVE.read_text(encoding="utf-8")

    # 1. Struct fields, inserted before `snapshot`.
    fields = "".join(
        f"    {field}: Option<Arc<dyn {port}>>,\n" for field, port, _, _, _ in DOMAINS
    )
    anchor = "    snapshot: Option<crate::os_control::capability::CapabilitySnapshot>,\n"
    if fields.strip().splitlines()[0] not in text:
        text = text.replace(anchor, fields + anchor, 1)

    # 2. compose_with initializers, inserted before the snapshot initializer.
    inits = "".join(f"            {field}: {expr},\n" for field, _, _, expr, _ in DOMAINS)
    init_anchor = "            snapshot,\n"
    if f"            {DOMAINS[0][0]}: " not in text:
        if init_anchor not in text:
            print("compose_with snapshot initializer not found", file=sys.stderr)
            return 1
        text = text.replace(init_anchor, inits + init_anchor, 1)

    # 3. Trait methods on the HostOsControl impl.
    methods = "".join(
        f"""
    fn {method}(&self) -> Option<&dyn {port}> {{
        self.{field}.as_deref()
    }}
"""
        for field, port, method, _, _ in DOMAINS
    )
    marker = "impl HostOsControl for LiveHostOsControl {"
    if f"    fn {DOMAINS[0][2]}(&self) -> Option<&dyn" not in text:
        text = text.replace(marker, marker + methods, 1)

    # 4. composed_domains entries.
    entries = "".join(
        f"""        if self.{field}.is_some() {{
            out.push("{label}");
        }}
"""
        for field, _, _, _, label in DOMAINS
    )
    domains_anchor = re.search(
        r"(pub fn composed_domains\(&self\) -> Vec<&'static str> \{\n\s*let mut out = Vec::new\(\);\n)",
        text,
    )
    if not domains_anchor:
        print("composed_domains not found", file=sys.stderr)
        return 1
    if f'out.push("{DOMAINS[0][4]}")' not in text:
        text = text.replace(domains_anchor.group(1), domains_anchor.group(1) + entries, 1)

    LIVE.write_text(text, encoding="utf-8")
    print(f"composed {len(DOMAINS)} domains into live.rs")
    return 0


if __name__ == "__main__":
    sys.exit(main())
