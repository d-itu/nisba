use std::{env, fs, path::PathBuf};

fn main() {
    let src = fs::read_to_string("json.nisba").unwrap();
    let out: PathBuf = env::var("OUT_DIR").unwrap().into();

    let contents = nisbac::generate(&src, nisbac::CodeGenKind::Encode).unwrap();
    fs::write(out.join("json_encode.rs"), contents).unwrap();

    let contents = nisbac::generate(&src, nisbac::CodeGenKind::Decode).unwrap();
    fs::write(out.join("json_decode.rs"), contents).unwrap();

    protobuf_codegen::CodeGen::new()
        .input("json.proto")
        .include("proto")
        .dependency(protobuf_well_known_types::get_dependency(
            "protobuf_well_known_types",
        ))
        .generate_and_compile()
        .unwrap();

    let dir: PathBuf = env::var("CARGO_MANIFEST_DIR").unwrap().into();
    let citm_catalog = dir.join("benches/data/citm_catalog.json");
    if !citm_catalog.exists() {
        let url = "https://raw.githubusercontent.com/miloyip/nativejson-benchmark/refs/tags/v1.0.0/data/citm_catalog.json";
        fs::write(
            citm_catalog,
            reqwest::blocking::get(url).unwrap().bytes().unwrap(),
        )
        .unwrap();
    }
}
