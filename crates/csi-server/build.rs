// The UI is built separately (`pnpm -C ui build`); make sure the embed folder exists so the
// server still compiles, serving a hint page instead of the app.
fn main() {
    let dist = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ui/dist");
    if !dist.join("index.html").exists() {
        std::fs::create_dir_all(&dist).unwrap();
        std::fs::write(
            dist.join("index.html"),
            "<!doctype html><title>csi</title><p>UI not built. Run <code>pnpm -C ui install && pnpm -C ui build</code>, then rebuild csi.</p>",
        )
        .unwrap();
    }
    println!("cargo:rerun-if-changed=../../ui/dist");
}
