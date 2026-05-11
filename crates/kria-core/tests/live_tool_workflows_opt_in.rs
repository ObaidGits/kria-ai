mod common;

use common::{internet_available, SandboxDir};
use kria_core::tools::registry;
use std::net::{TcpStream, ToSocketAddrs};
use std::process::Command;
use std::time::Duration;

fn env_flag(name: &str) -> bool {
    std::env::var(name)
        .map(|v| {
            let lower = v.trim().to_ascii_lowercase();
            matches!(lower.as_str(), "1" | "true" | "yes" | "on")
        })
        .unwrap_or(false)
}

fn live_suite_enabled() -> bool {
    env_flag("KRIA_LIVE_TOOL_SUITE")
}

fn workflow_enabled(flag: &str) -> bool {
    live_suite_enabled() && env_flag(flag)
}

fn command_available(name: &str) -> bool {
    Command::new("which")
        .arg(name)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn package_manager_binary(source: &str) -> Option<&'static str> {
    match source {
        "apt" => Some("apt-get"),
        "dnf" => Some("dnf"),
        "pacman" => Some("pacman"),
        "zypper" => Some("zypper"),
        "brew" => Some("brew"),
        "winget" => Some("winget"),
        "choco" => Some("choco"),
        "snap" => Some("snap"),
        "flatpak" => Some("flatpak"),
        _ => None,
    }
}

fn source_requires_privileged_install(source: &str) -> bool {
    matches!(source, "apt" | "dnf" | "pacman" | "zypper" | "snap")
}

fn sudo_non_interactive_ready() -> bool {
    Command::new("sudo")
        .args(["-n", "true"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn tcp_probe(endpoint: &str, timeout: Duration) -> bool {
    let Ok(addrs) = endpoint.to_socket_addrs() else {
        return false;
    };

    addrs
        .into_iter()
        .any(|addr| TcpStream::connect_timeout(&addr, timeout).is_ok())
}

fn balanced_three_host_probe() -> (bool, Vec<(String, bool)>) {
    let endpoints = ["1.1.1.1:80", "8.8.8.8:53", "example.com:443"];
    let mut statuses = Vec::with_capacity(endpoints.len());

    for endpoint in endpoints {
        let ok = tcp_probe(endpoint, Duration::from_secs(3));
        statuses.push((endpoint.to_string(), ok));
    }

    let ok_count = statuses.iter().filter(|(_, ok)| *ok).count();
    (ok_count >= 2, statuses)
}

#[tokio::test]
async fn live_install_workflow_opt_in_real_install() {
    if !workflow_enabled("KRIA_LIVE_TOOL_INSTALL") {
        eprintln!(
            "SKIP: set KRIA_LIVE_TOOL_SUITE=1 and KRIA_LIVE_TOOL_INSTALL=1 to enable install workflow live test"
        );
        return;
    }

    let package = match std::env::var("KRIA_LIVE_INSTALL_PACKAGE") {
        Ok(value) if !value.trim().is_empty() => value.trim().to_string(),
        _ => {
            eprintln!("SKIP: KRIA_LIVE_INSTALL_PACKAGE is required for install workflow");
            return;
        }
    };

    let source = match std::env::var("KRIA_LIVE_INSTALL_SOURCE") {
        Ok(value) if !value.trim().is_empty() => value.trim().to_ascii_lowercase(),
        _ => {
            eprintln!("SKIP: KRIA_LIVE_INSTALL_SOURCE is required for install workflow");
            return;
        }
    };

    let Some(pm_bin) = package_manager_binary(&source) else {
        eprintln!("SKIP: unsupported KRIA_LIVE_INSTALL_SOURCE='{source}'");
        return;
    };

    if !command_available(pm_bin) {
        eprintln!("SKIP: required package manager binary '{pm_bin}' is not available");
        return;
    }

    if !internet_available() {
        eprintln!("SKIP: internet preflight failed");
        return;
    }

    if source_requires_privileged_install(&source) {
        if !env_flag("KRIA_LIVE_INSTALL_ALLOW_PRIVILEGED") {
            eprintln!(
                "SKIP: privileged install source '{source}' requires KRIA_LIVE_INSTALL_ALLOW_PRIVILEGED=1"
            );
            return;
        }

        if command_available("pkexec") {
            eprintln!(
                "SKIP: pkexec detected; live test avoids interactive auth prompts. Use a non-privileged source or run where pkexec is unavailable"
            );
            return;
        }

        if !sudo_non_interactive_ready() {
            eprintln!("SKIP: sudo -n preflight failed for privileged install source '{source}'");
            return;
        }
    }

    let reg = registry::build_default_registry();
    let search = reg
        .get_handler("search_package")
        .expect("search_package handler missing")
        .clone();
    let check = reg
        .get_handler("check_package_installed")
        .expect("check_package_installed handler missing")
        .clone();
    let install = reg
        .get_handler("install_package")
        .expect("install_package handler missing")
        .clone();

    let search_result = search
        .execute(serde_json::json!({
            "query": package,
            "source": source,
        }))
        .await;
    assert!(
        search_result.success,
        "search_package failed unexpectedly: {:?}",
        search_result.error
    );

    let before = check.execute(serde_json::json!({ "name": package })).await;
    assert!(
        before.success,
        "check_package_installed preflight call failed: {:?}",
        before.error
    );

    let install_result = install
        .execute(serde_json::json!({
            "name": package,
            "source": source,
        }))
        .await;
    assert!(
        install_result.success,
        "install_package failed unexpectedly: {:?}",
        install_result.error
    );
    assert_eq!(
        install_result.data["success"].as_bool(),
        Some(true),
        "install_package must report success=true in payload"
    );

    let after = check.execute(serde_json::json!({ "name": package })).await;
    assert!(
        after.success,
        "check_package_installed post-install call failed: {:?}",
        after.error
    );
    assert_eq!(
        after.data["installed"].as_bool(),
        Some(true),
        "target package should be installed after live install workflow"
    );
}

#[tokio::test]
async fn live_open_workflow_opt_in_real_application_launch() {
    if !workflow_enabled("KRIA_LIVE_TOOL_OPEN") {
        eprintln!(
            "SKIP: set KRIA_LIVE_TOOL_SUITE=1 and KRIA_LIVE_TOOL_OPEN=1 to enable open workflow live test"
        );
        return;
    }

    let app_name = std::env::var("KRIA_LIVE_OPEN_APP")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| "true".to_string());

    if !command_available(&app_name) {
        eprintln!("SKIP: application binary '{app_name}' not available in PATH");
        return;
    }

    let reg = registry::build_default_registry();
    let open_application = reg
        .get_handler("open_application")
        .expect("open_application handler missing")
        .clone();

    let result = open_application
        .execute(serde_json::json!({
            "name": app_name,
            "args": [],
        }))
        .await;

    assert!(
        result.success,
        "open_application failed unexpectedly: {:?}",
        result.error
    );
    assert_eq!(
        result.data["launched"].as_bool(),
        Some(true),
        "open_application should report launched=true"
    );
}

#[tokio::test]
async fn live_network_workflow_opt_in_real_fetch_and_download() {
    if !workflow_enabled("KRIA_LIVE_TOOL_NETWORK") {
        eprintln!(
            "SKIP: set KRIA_LIVE_TOOL_SUITE=1 and KRIA_LIVE_TOOL_NETWORK=1 to enable network workflow live test"
        );
        return;
    }

    let (probe_ok, statuses) = balanced_three_host_probe();
    if !probe_ok {
        eprintln!("SKIP: balanced three-host probe failed: {statuses:?}");
        return;
    }

    if !internet_available() {
        eprintln!("SKIP: internet preflight failed");
        return;
    }

    let reg = registry::build_default_registry();
    let check_url = reg
        .get_handler("check_url_status")
        .expect("check_url_status handler missing")
        .clone();
    let fetch_page = reg
        .get_handler("fetch_webpage")
        .expect("fetch_webpage handler missing")
        .clone();
    let download_file = reg
        .get_handler("download_file")
        .expect("download_file handler missing")
        .clone();

    let url = "https://example.com";

    let status_result = check_url.execute(serde_json::json!({ "url": url })).await;
    assert!(
        status_result.success,
        "check_url_status failed unexpectedly: {:?}",
        status_result.error
    );
    assert_eq!(
        status_result.data["reachable"].as_bool(),
        Some(true),
        "example.com should be reachable for live network workflow"
    );

    let fetch_result = fetch_page
        .execute(serde_json::json!({
            "url": url,
            "max_chars": 512,
        }))
        .await;
    assert!(
        fetch_result.success,
        "fetch_webpage failed unexpectedly: {:?}",
        fetch_result.error
    );
    assert!(
        fetch_result.data["content"]
            .as_str()
            .map(|text| !text.trim().is_empty())
            .unwrap_or(false),
        "fetch_webpage should return non-empty content"
    );

    let sandbox = SandboxDir::new();
    let destination = sandbox.child("downloads/example.html");
    let download_result = download_file
        .execute(serde_json::json!({
            "url": url,
            "destination": destination.to_string_lossy(),
            "max_size_mb": 2,
        }))
        .await;
    assert!(
        download_result.success,
        "download_file failed unexpectedly: {:?}",
        download_result.error
    );
    assert!(
        destination.exists(),
        "download_file should create destination file"
    );
    assert!(
        download_result.data["size_bytes"].as_u64().unwrap_or(0) > 0,
        "download_file should report downloaded bytes"
    );
}
