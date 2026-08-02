use mitsuro_core::{identity::CONFIG_DIR_NAME, paths};

#[test]
fn integration_tests_do_not_resolve_the_real_user_config_root() {
    let config = paths::config_dir();

    assert!(config.starts_with(std::env::temp_dir().join("mitsuro-cargo-tests")));
    assert_eq!(
        config.file_name(),
        Some(std::ffi::OsStr::new(CONFIG_DIR_NAME))
    );
    if let Some(home) = dirs::home_dir() {
        assert_ne!(config, home.join(CONFIG_DIR_NAME));
    }
}
