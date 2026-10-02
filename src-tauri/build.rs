fn main() {
    // Windows : la boîte de dialogue native (rfd, via tauri-plugin-dialog) exige le manifeste
    // « Common Controls v6 ». tauri-build ne l'embarque que dans l'exécutable final : les binaires de
    // test (`cargo test`) planteraient au démarrage avec STATUS_ENTRYPOINT_NOT_FOUND (0xc0000139).
    // On embarque donc le manifeste nous-mêmes dans tous les artefacts, et on désactive celui de
    // tauri-build pour ne pas en avoir deux dans l'exécutable.
    let windows = tauri_build::WindowsAttributes::new_without_app_manifest();
    tauri_build::try_build(tauri_build::Attributes::new().windows_attributes(windows))
        .expect("échec de tauri-build");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        println!("cargo:rerun-if-changed=windows-manifest.rc");
        println!("cargo:rerun-if-changed=windows-app-manifest.xml");
        embed_resource::compile_for_everything("windows-manifest.rc", embed_resource::NONE)
            .manifest_required()
            .expect("échec de l'embarquement du manifeste Windows");
    }
}
