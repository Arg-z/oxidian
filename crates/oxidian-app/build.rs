fn main() {
    // frontendDist berubah → codegen asset harus regenerate
    println!("cargo:rerun-if-changed=../../ui");
    tauri_build::build()
}
