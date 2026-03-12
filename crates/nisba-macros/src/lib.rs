extern crate proc_macro;
use std::{env, fs, path::PathBuf};

use nisbac::{
    back::{CodeGenKind, rust::Config},
    schema::Schema,
};
use proc_macro::TokenStream;

fn get_schema(input: TokenStream) -> Schema {
    let root = env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".into());
    let root = PathBuf::from(root);
    let path: syn::LitStr = syn::parse(input).unwrap();
    let path = path.value();
    let path = root.join(path);
    let s = fs::read_to_string(&path).unwrap();
    let ast = nisbac::ast::parse(&s).unwrap();
    Schema::from_ast(ast).unwrap()
}

#[proc_macro]
pub fn generate_encode(input: TokenStream) -> TokenStream {
    nisbac::back::rust::generate(&get_schema(input), CodeGenKind::Encode, Config {}).into()
}

#[proc_macro]
pub fn generate_decode(input: TokenStream) -> TokenStream {
    nisbac::back::rust::generate(&get_schema(input), CodeGenKind::Decode, Config {}).into()
}
