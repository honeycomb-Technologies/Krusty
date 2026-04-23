use super::check::is_newer_version;
use super::paths::detect_platform;

#[test]
fn semver_comparison_prefers_higher_components() {
    assert!(is_newer_version("0.4.2", "0.4.1"));
    assert!(is_newer_version("1.0.0", "0.999.999"));
    assert!(!is_newer_version("0.4.1", "0.4.1"));
    assert!(!is_newer_version("0.4.0", "0.4.1"));
}

#[test]
fn semver_comparison_handles_missing_parts_as_zero() {
    assert!(is_newer_version("0.5", "0.4.9"));
    assert!(is_newer_version("2", "1.99.99"));
    assert!(!is_newer_version("0.4", "0.4.1"));
}

#[test]
fn detect_platform_matches_runtime_target() {
    let expected = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => Some("x86_64-unknown-linux-gnu"),
        ("linux", "aarch64") => Some("aarch64-unknown-linux-gnu"),
        ("macos", "x86_64") => Some("x86_64-apple-darwin"),
        ("macos", "aarch64") => Some("aarch64-apple-darwin"),
        ("windows", "x86_64") => Some("x86_64-pc-windows-msvc"),
        _ => None,
    };

    match expected {
        Some(expected) => assert_eq!(detect_platform().unwrap(), expected),
        None => assert!(detect_platform().is_err()),
    }
}
