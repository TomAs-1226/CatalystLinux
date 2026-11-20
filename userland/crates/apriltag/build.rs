//! Builds the AprilTag library (AprilRobotics/apriltag 3.4.5, BSD 2-Clause,
//! vendored under `c/`: the detector, its pose estimation, the 36h11 family
//! and the common code) and the flat shim the Rust side calls, for whatever
//! this is compiled for: the handheld's ARM, or the computer the simulator
//! runs on.

fn main() {
    let dir = std::path::Path::new("c");
    let mut files = vec![
        dir.join("apriltag.c"),
        dir.join("apriltag_pose.c"),
        dir.join("apriltag_quad_thresh.c"),
        dir.join("tag36h11.c"),
        dir.join("catalyst_shim.c"),
    ];
    for entry in std::fs::read_dir(dir.join("common")).expect("the vendored sources") {
        let path = entry.expect("a source file").path();
        if path.extension().is_some_and(|e| e == "c") {
            files.push(path);
        }
    }
    println!("cargo:rerun-if-changed=c");
    let mut build = cc::Build::new();
    build.files(&files).include(dir).include(dir.join("common")).warnings(false).opt_level(2);
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        // As upstream's CMake builds it on Windows.
        build.define("WIN32_LEAN_AND_MEAN", None).define("_CRT_SECURE_NO_WARNINGS", None).define("_CRT_NONSTDC_NO_DEPRECATE", None);
    }
    build.compile("apriltag");
}
