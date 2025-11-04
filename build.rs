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
    slint_build::compile("src/ui/main.slint")
        .expect("Failed to compile Slint UI");

    // After this runs successfully:
    // - Slint generates Rust bindings for MainWindow component
    // - We can use it in main.rs like: slint::include_modules!();

    println!("cargo:rerun-if-changed=src/ui/main.slint");
    // This tells Cargo: "If main.slint changes, run build.rs again"
    // Like file watching in webpack or nodemon
}
