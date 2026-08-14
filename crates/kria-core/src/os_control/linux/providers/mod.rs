//! Live Linux per-domain provider adapters (design §3 `linux/providers/*`).
//!
//! Each adapter here is a **live** integration that talks to a real host service
//! (D-Bus, Secret Service, UDisks2, …). Per Task 0.4 every such constructor is a
//! raw transport: it requires a [`crate::os_control::access::LiveHostAccessToken`]
//! (mintable only in a live composition root) and trips the process-wide
//! deny-live sentinel before touching the host, so no `os-control-test`
//! completion binary can reach a live provider. Deny-live tests inject the
//! module-level fakes instead (e.g.
//! [`crate::os_control::secrets::FakeCredentialStore`]).
//!
//! Providers land incrementally across Tasks 1.x–3.x. Task **1.10** adds the
//! freedesktop Secret Service adapter seam ([`secret_service`]); its live D-Bus
//! wiring is completed when connectivity integrates the credential store in
//! Task 3.10.

/// The shared read/mutate seam every CLI-backed live provider uses.
pub mod cli_query;

/// The live display-configuration provider.
pub mod display_config;

/// The live print provider, backed by CUPS.
pub mod cups_print;

/// The live privacy-control and firewall providers.
pub mod privacy_firewall;

/// The live backup and document-scan provider.
pub mod backup_scan;

/// The live firmware-awareness and hardware-sensor providers.
pub mod firmware_sensors;

/// The live desktop-search provider, backed by GNOME Tracker 3.
pub mod tracker_search;

/// The live system-health provider: diagnostics, logs and recipes.
pub mod system_health;

pub mod application_control;
pub mod bluez;
pub mod automation;
pub mod clipboard;
pub mod files;
pub mod gnome_display;
pub mod logind;
pub mod network_manager;
pub mod notifications;
pub mod packagekit;
pub mod pipewire;
pub mod power_profiles;
pub mod process_control;
pub mod secret_service;
pub mod udisks;
