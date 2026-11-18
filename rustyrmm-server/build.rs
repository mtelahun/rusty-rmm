//! Release builds must include the web UI; debug builds may serve a placeholder.
fn main() {
    let index = std::path::Path::new("../rustyrmm-web/dist/index.html");
    println!("cargo:rerun-if-changed=../rustyrmm-web/dist");
    if std::env::var("PROFILE").as_deref() == Ok("release") && !index.exists() {
        panic!(
            "release builds embed the web UI, but rustyrmm-web/dist is missing: \
             run `npm ci && npm run build` in rustyrmm-web/ first"
        );
    }
}
