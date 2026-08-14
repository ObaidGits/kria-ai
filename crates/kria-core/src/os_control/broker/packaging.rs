//! Polkit action/policy packaging for the six broker operations.
//!
//! linux-os-control-production **Task 1.5**, design §12
//! (OSC-004, OSC-030, OSC-033).
//!
//! This module owns the mapping from each closed [`BrokerOperation`] to its
//! registered Polkit action id, and a **pure parser** over the embedded
//! `org.kria.broker.policy` file. The parser only reads text; it never invokes
//! `pkexec`, `polkit`, or any privileged process (OSC-033), so the packaging is
//! validated at test time with zero live authority.
//!
//! The completion proof is structural: the policy registers **exactly** the six
//! broker actions and no others, so no additional privileged action can be
//! smuggled into the packaging.

use super::protocol::BrokerOperation;

/// The embedded Polkit policy document (validated by [`parse_policy_actions`]).
pub const BROKER_POLKIT_POLICY: &str = include_str!("packaging/org.kria.broker.policy");

/// The Polkit action id for [`BrokerOperation::ApplyPackagePlan`].
pub const ACTION_APPLY_PACKAGE_PLAN: &str = "org.kria.broker.apply-package-plan";
/// The Polkit action id for [`BrokerOperation::SetBoundPathOwnership`].
pub const ACTION_SET_BOUND_PATH_OWNERSHIP: &str = "org.kria.broker.set-bound-path-ownership";
/// The Polkit action id for [`BrokerOperation::SetFirewallEnabled`].
pub const ACTION_SET_FIREWALL_ENABLED: &str = "org.kria.broker.set-firewall-enabled";
/// The Polkit action id for [`BrokerOperation::SetPrivacyControl`].
pub const ACTION_SET_PRIVACY_CONTROL: &str = "org.kria.broker.set-privacy-control";
/// The Polkit action id for [`BrokerOperation::ConfigureDiscoveredPrinter`].
pub const ACTION_CONFIGURE_DISCOVERED_PRINTER: &str =
    "org.kria.broker.configure-discovered-printer";
/// The Polkit action id for [`BrokerOperation::SetBatteryChargeThresholds`].
pub const ACTION_SET_BATTERY_CHARGE_THRESHOLDS: &str =
    "org.kria.broker.set-battery-charge-thresholds";

/// The complete, closed set of registered broker action ids, in operation-tag
/// order.
pub const BROKER_ACTION_IDS: [&str; BrokerOperation::COUNT] = [
    ACTION_APPLY_PACKAGE_PLAN,
    ACTION_SET_BOUND_PATH_OWNERSHIP,
    ACTION_SET_FIREWALL_ENABLED,
    ACTION_SET_PRIVACY_CONTROL,
    ACTION_CONFIGURE_DISCOVERED_PRINTER,
    ACTION_SET_BATTERY_CHARGE_THRESHOLDS,
];

/// The registered Polkit action id for an operation (never model prose).
#[must_use]
pub fn polkit_action_id(operation: &BrokerOperation) -> &'static str {
    match operation {
        BrokerOperation::ApplyPackagePlan { .. } => ACTION_APPLY_PACKAGE_PLAN,
        BrokerOperation::SetBoundPathOwnership { .. } => ACTION_SET_BOUND_PATH_OWNERSHIP,
        BrokerOperation::SetFirewallEnabled { .. } => ACTION_SET_FIREWALL_ENABLED,
        BrokerOperation::SetPrivacyControl { .. } => ACTION_SET_PRIVACY_CONTROL,
        BrokerOperation::ConfigureDiscoveredPrinter { .. } => ACTION_CONFIGURE_DISCOVERED_PRINTER,
        BrokerOperation::SetBatteryChargeThresholds { .. } => ACTION_SET_BATTERY_CHARGE_THRESHOLDS,
    }
}

/// Extract every `action id="…"` value from the Polkit policy text. This is a
/// pure text scan — it launches no process.
#[must_use]
pub fn parse_policy_actions(policy_xml: &str) -> Vec<String> {
    let mut ids = Vec::new();
    let needle = "<action id=\"";
    let mut rest = policy_xml;
    while let Some(start) = rest.find(needle) {
        let after = &rest[start + needle.len()..];
        if let Some(end) = after.find('"') {
            ids.push(after[..end].to_string());
            rest = &after[end..];
        } else {
            break;
        }
    }
    ids
}

#[cfg(all(test, feature = "os-control-test"))]
mod tests {
    use super::*;
    use crate::os_control::access::{sentinel_is_armed, sentinel_trip_count};

    #[test]
    fn policy_registers_exactly_the_six_broker_actions() {
        // Pure parse: prove the sentinel is armed and no live process is started.
        assert!(sentinel_is_armed());
        let before = sentinel_trip_count();

        let ids = parse_policy_actions(BROKER_POLKIT_POLICY);
        assert_eq!(ids.len(), BrokerOperation::COUNT, "exactly six actions");

        let mut sorted = ids.clone();
        sorted.sort();
        let mut expected: Vec<String> =
            BROKER_ACTION_IDS.iter().map(|s| (*s).to_string()).collect();
        expected.sort();
        assert_eq!(sorted, expected, "no extra or missing privileged action");

        // No process/bus/polkit transport was opened by parsing.
        assert_eq!(sentinel_trip_count(), before);
    }

    #[test]
    fn every_operation_maps_to_a_registered_action() {
        use crate::os_control::broker::protocol::*;
        use crate::os_control::contract::{Digest, NonEmptyBoundedVec, SafeText};

        let ops = [
            BrokerOperation::ApplyPackagePlan {
                provider: PackageProviderId::Apt,
                approved_plan_digest: Digest::of_str("p"),
                transaction: BoundedPackageTransaction::new(NonEmptyBoundedVec::single(
                    PackageStep {
                        action: PackageStepAction::Install,
                        package: BoundedPackageName::new("pkg").unwrap(),
                    },
                )),
            },
            BrokerOperation::SetBoundPathOwnership {
                path: BrokerBoundPath {
                    path: "/x".into(),
                    device: 1,
                    inode: 1,
                    owner_uid: 0,
                },
                owner: ExistingLocalIdentity {
                    uid: 0,
                    name: SafeText::new("root"),
                },
            },
            BrokerOperation::SetFirewallEnabled {
                provider: FirewallProviderId::Ufw,
                enabled: true,
            },
            BrokerOperation::SetPrivacyControl {
                control: RecognizedPrivacyControl::CameraAccess,
                enabled: false,
            },
            BrokerOperation::ConfigureDiscoveredPrinter {
                printer: DiscoveredPrinterId::new("p").unwrap(),
                options: ReviewedPrinterOptions {
                    set_default: false,
                    shared: false,
                    accept_jobs: true,
                },
            },
            BrokerOperation::SetBatteryChargeThresholds {
                adapter: ChargeThresholdAdapterId::SysfsStandard,
                lower_percent: BoundedPercent::new(50).unwrap(),
                upper_percent: BoundedPercent::new(90).unwrap(),
            },
        ];

        let registered = parse_policy_actions(BROKER_POLKIT_POLICY);
        for op in ops {
            let id = polkit_action_id(&op);
            assert!(
                registered.iter().any(|r| r == id),
                "operation {} maps to unregistered action {id}",
                op.token()
            );
        }
    }
}
