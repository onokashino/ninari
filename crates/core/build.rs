fn main() {
    // The Shikimori app keys: from the environment, or from a git-ignored .env.local at the
    // project root with SHIKIMORI_CLIENT_ID=... and SHIKIMORI_CLIENT_SECRET=... lines.
    // A .env.local created after the last build is seen only once build.rs is touched.
    println!("cargo:rerun-if-env-changed=SHIKIMORI_CLIENT_ID");
    println!("cargo:rerun-if-env-changed=SHIKIMORI_CLIENT_SECRET");
    let file = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.env.local");
    if let Ok(text) = std::fs::read_to_string(&file) {
        println!("cargo:rerun-if-changed={}", file.display());
        for (key, value) in text.lines().filter_map(|l| l.split_once('=')) {
            let key = key.trim();
            if key.starts_with("SHIKIMORI_") && std::env::var(key).is_err() {
                println!("cargo:rustc-env={key}={}", value.trim());
            }
        }
    }
}
