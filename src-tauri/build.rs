use std::path::Path;
use std::process::Command;

fn main() {
    link_clang_rt();
    tauri_build::build()
}

/// Link clang's runtime library, which provides `__isPlatformVersionAtLeast`.
///
/// ggml's Metal backend guards newer Metal APIs with `@available`. When the
/// deployment target is older than the SDK (`tauri build` targets macOS 11),
/// clang turns those checks into calls into `libclang_rt.osx.a`. rustc does
/// not link that library, so the release build fails with an undefined symbol.
fn link_clang_rt() {
    let output = Command::new("clang")
        .arg("--print-resource-dir")
        .output()
        .expect("could not run `clang --print-resource-dir`");
    assert!(
        output.status.success(),
        "`clang --print-resource-dir` failed"
    );
    let resource_dir = String::from_utf8(output.stdout).expect("clang resource dir is UTF-8");
    let dir = Path::new(resource_dir.trim()).join("lib/darwin");
    assert!(
        dir.join("libclang_rt.osx.a").exists(),
        "libclang_rt.osx.a not found in {}",
        dir.display()
    );
    println!("cargo:rustc-link-search=native={}", dir.display());
    println!("cargo:rustc-link-lib=static=clang_rt.osx");
}
