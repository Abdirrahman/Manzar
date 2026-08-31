//! Records the compiler that built the rig, so a report always says which
//! toolchain produced its numbers.

fn main() {
    let version = std::process::Command::new(std::env::var("RUSTC").unwrap_or("rustc".into()))
        .arg("--version")
        .output()
        .ok()
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .unwrap_or_else(|| "unknown".into());

    println!("cargo:rustc-env=BENCH_RUSTC_VERSION={}", version.trim());
    println!("cargo:rerun-if-changed=build.rs");
}
