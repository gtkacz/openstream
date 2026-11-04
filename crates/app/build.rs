fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=brp.rc");
    println!("cargo:rerun-if-changed=assets/icon/openstream.ico");
    // Does nothing unless the target is Windows. `optional` only excuses a cross-build with no
    // resource compiler: on Windows itself a missing `rc.exe` still fails, so releases keep the icon.
    embed_resource::compile("brp.rc", embed_resource::NONE)
        .manifest_optional()
        .unwrap();
}
