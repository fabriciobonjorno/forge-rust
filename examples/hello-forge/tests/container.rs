//! Keeps the container build aligned with the local toolchain.

use std::fs;

#[test]
fn dockerfile_rust_version_matches_rust_toolchain() -> Result<(), Box<dyn std::error::Error>> {
    let root = env!("CARGO_MANIFEST_DIR");
    let toolchain = fs::read_to_string(format!("{root}/rust-toolchain.toml"))?;
    let dockerfile = fs::read_to_string(format!("{root}/Dockerfile"))?;

    let channel = value_after(&toolchain, "channel = ").map(|value| value.trim_matches('"'));
    let image = value_after(&dockerfile, "ARG RUST_VERSION=");

    assert!(channel.is_some(), "rust-toolchain.toml must pin a channel");
    assert_eq!(
        channel, image,
        "Dockerfile RUST_VERSION must equal the rust-toolchain.toml channel"
    );
    Ok(())
}

fn value_after<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    text.lines()
        .find_map(|line| line.trim().strip_prefix(prefix))
        .map(str::trim)
}
