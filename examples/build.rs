use std::{env, fs, path::PathBuf};

fn main() {
    let src = fs::read_to_string("src/json.nisba").unwrap();

    let contents = nisbac::generate(&src, nisbac::CodeGenKind::Encode).unwrap();
    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    fs::write(out.join("json_encode.rs"), contents).unwrap();

    let contents = nisbac::generate(&src, nisbac::CodeGenKind::Decode).unwrap();
    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    fs::write(out.join("json_decode.rs"), contents).unwrap();
}
