// SDL3 link helper: the host may only have the runtime library (libSDL3.so.0) without the
// unversioned development symlink or a pkg-config file. Create a symlink in OUT_DIR so the
// linker can resolve `-lSDL3`.
use std::path::Path;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=SDL3_LIB_DIR");
    let out = std::env::var("OUT_DIR").unwrap();
    let link = Path::new(&out).join("libSDL3.so");
    let candidates: Vec<String> = {
        let mut v = Vec::new();
        if let Ok(d) = std::env::var("SDL3_LIB_DIR") {
            v.push(format!("{d}/libSDL3.so"));
            v.push(format!("{d}/libSDL3.so.0"));
        }
        for d in ["/usr/lib64", "/usr/lib", "/usr/lib/x86_64-linux-gnu", "/usr/local/lib64", "/usr/local/lib"] {
            v.push(format!("{d}/libSDL3.so.0"));
            v.push(format!("{d}/libSDL3.so"));
        }
        v
    };
    for c in candidates {
        let p = Path::new(&c);
        if p.exists() {
            let _ = std::fs::remove_file(&link);
            if std::os::unix::fs::symlink(p, &link).is_ok() {
                println!("cargo:rustc-link-search=native={out}");
            }
            break;
        }
    }
}
