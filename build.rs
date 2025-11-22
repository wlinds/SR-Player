// Build script for Cargo
//
// WHAT IS build.rs?
// In Rust, build.rs runs BEFORE your main code compiles.
// It's like a "pre-build" step or "webpack configuration" in JavaScript.

fn main() {
    // slint_build::compile() is provided by the 'slint-build' crate
    // It finds and compiles all .slint files in the specified path

    // This compiles src/ui/main.slint into Rust code
    // The generated code will be available as a module in our program

    // The expect() method is like try-catch in JavaScript:
    // If compilation fails, panic with this error message
    slint_build::compile("src/ui/main.slint").expect("Failed to compile Slint UI");

    // After this runs successfully:
    // - Slint generates Rust bindings for MainWindow component
    // - We can use it in main.rs like: slint::include_modules!();

    println!("cargo:rerun-if-changed=src/ui/main.slint");
    println!("cargo:rerun-if-changed=src/ui/types.slint");
    println!("cargo:rerun-if-changed=src/ui/now_playing.slint");
    println!("cargo:rerun-if-changed=src/ui/tab_content.slint");
    println!("cargo:rerun-if-changed=src/ui/tab_bar.slint");
    // This tells Cargo: "If main.slint changes, run build.rs again"
    // Like file watching in webpack or nodemon

    // Embed Windows application icon and metadata
    // Note: We check CARGO_CFG_TARGET_OS instead of cfg!(target_os) because
    // build.rs runs on the host, not the target
    if std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default() == "windows" {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("wix/Product.ico");
        res.set("ProductName", "SR Player");
        res.set("FileDescription", "Sveriges Radio Streaming Application");
        res.compile().expect("Failed to compile Windows resources");
    }
}
