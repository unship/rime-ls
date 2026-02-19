extern crate bindgen;

use std::env;
use std::path::PathBuf;

/// Find Homebrew-installed librime. Returns (include_dir, lib_dir) or None.
fn find_homebrew_librime() -> Option<(PathBuf, PathBuf)> {
    // Try brew --prefix librime first
    if let Ok(output) = std::process::Command::new("brew")
        .args(["--prefix", "librime"])
        .output()
    {
        if output.status.success() {
            let prefix = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !prefix.is_empty() {
                let include = PathBuf::from(&prefix).join("include");
                let lib = PathBuf::from(&prefix).join("lib");
                if include.exists() && lib.exists() {
                    return Some((include, lib));
                }
            }
        }
    }

    // Check common Homebrew paths
    let candidates = [
        "/opt/homebrew/opt/librime",  // Apple Silicon
        "/usr/local/opt/librime",    // Intel Mac
    ];

    for prefix in candidates {
        let include = PathBuf::from(prefix).join("include");
        let lib = PathBuf::from(prefix).join("lib");
        if include.exists() && lib.exists() {
            return Some((include, lib));
        }
    }

    None
}

fn main() {
    let (librime_include_dir, librime_lib_dir) =
        if let (Ok(include), Ok(lib)) = (
            env::var("LIBRIME_INCLUDE_DIR"),
            env::var("LIBRIME_LIB_DIR"),
        ) {
            (PathBuf::from(include), PathBuf::from(lib))
        } else if let Some((include, lib)) = find_homebrew_librime() {
            (include, lib)
        } else {
            (
                PathBuf::from("librime/dist/include"),
                PathBuf::from("librime/dist/lib"),
            )
        };

    let librime_include_dir = librime_include_dir.to_string_lossy();
    let librime_lib_dir = librime_lib_dir.to_string_lossy();

    println!("cargo:rustc-link-search={librime_lib_dir}");
    // Embed rpath so the binary finds librime at runtime (macOS/Linux)
    let target = env::var("TARGET").unwrap_or_default();
    if target.contains("darwin") || target.contains("linux") {
        println!("cargo:rustc-link-arg=-Wl,-rpath,{librime_lib_dir}");
    }
    println!("cargo:rustc-link-lib=rime");
    if env::var("CARGO_FEATURE_SEPARATE_GEARS_LIB").is_ok() {
        println!("cargo:rustc-link-lib=rime-gears");
    }

    let bindings = bindgen::Builder::default()
        .header("wrapper.h")
        .clang_arg(format!("-I{librime_include_dir}"))
        .generate()
        .expect("Unable to generate bindings");

    let out_path = PathBuf::from(env::var("OUT_DIR").unwrap());
    bindings
        .write_to_file(out_path.join("bindings.rs"))
        .expect("Couldn't write bindings!");
}
