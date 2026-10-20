fn main() {
    println!("cargo:rerun-if-changed=resources/nuvio.rc");
    println!("cargo:rerun-if-changed=resources/nuvio.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        // The icon in Explorer and the taskbar, and the version block shown in the file's properties.
        embed_resource::compile("resources/nuvio.rc", embed_resource::NONE)
            .manifest_required()
            .expect("could not embed the Windows resources");
    }
}
