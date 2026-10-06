fn main() {
    // Expone el target triple de compilación al código (lo usa el updater para
    // elegir el binario correcto del release).
    let target = std::env::var("TARGET").unwrap_or_else(|_| String::from("unknown"));
    println!("cargo:rustc-env=RUSTIDIAN_TARGET={target}");

    slint_build::compile("ui/main.slint").unwrap();
}
