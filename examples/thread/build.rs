use std::collections::HashMap;

fn main() {
    let libraries = HashMap::from([("assistant".to_string(), ai_ui_slint::ui_dir())]);
    // The software renderer needs resources embedded as pixels and glyphs; the
    // GPU renderers load them as files. `--features gpu-window` builds for the
    // latter (offscreen `--screenshot` then needs the default build).
    let kind = if std::env::var_os("CARGO_FEATURE_GPU_WINDOW").is_some() {
        slint_build::EmbedResourcesKind::EmbedFiles
    } else {
        slint_build::EmbedResourcesKind::EmbedForSoftwareRenderer
    };
    if kind == slint_build::EmbedResourcesKind::EmbedForSoftwareRenderer
        && std::env::var_os("SLINT_FONT_SIZES").is_none()
    {
        // The software renderer draws pre-rendered glyphs at the sizes it can
        // see in the source. Text size multiplies those sizes at run time, so
        // embed the sizes the four text scales (0.9 to 1.3) produce from the
        // thread's 12 to 24 px text, or a scale would snap to the nearest
        // size already there. GPU renderers draw any size from the font file.
        std::env::set_var("SLINT_FONT_SIZES", "10,15,17,18,21,22,23,26,28,31");
    }
    let config = slint_build::CompilerConfiguration::new()
        .with_library_paths(libraries)
        .embed_resources(kind);
    slint_build::compile_with_config("ui/app.slint", config).unwrap();
}
