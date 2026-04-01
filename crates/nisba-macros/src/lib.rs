extern crate proc_macro;
use std::{error, fmt::Display, fs, path::PathBuf};

use nisbac::{
    back::{CodeGenKind, rust::Config},
    parser,
    schema::{self, Validated},
};
use proc_macro::{Span, TokenStream};
use proc_macro_error::proc_macro_error;

#[derive(Debug)]
struct Error(String);

impl Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl error::Error for Error {}

fn get_schema(path: String) -> Result<Validated, Box<dyn error::Error>> {
    let path = PathBuf::from(path);
    let path = if path.is_absolute() {
        path
    } else {
        let file = PathBuf::from(Span::call_site().file());
        file.exists()
            .then_some(file)
            .unwrap_or_default()
            .with_file_name(path)
    };

    let s = fs::read_to_string(&path).map_err(|e| {
        Error({
            let mut s = path.to_str().map(|x| x.to_string()).unwrap_or_default();
            s.push_str(&e.to_string());
            s
        })
    })?;
    let ast = parser::parse(&s)?;
    let schema = schema::resolve(&ast)?;
    Ok(schema::validate(schema)?)
}

#[proc_macro_error]
#[proc_macro]
pub fn generate_encode(input: TokenStream) -> TokenStream {
    let path: syn::LitStr = match syn::parse(input) {
        Ok(x) => x,
        Err(e) => proc_macro_error::abort!(e.span(), e),
    };
    match get_schema(path.value()) {
        Ok(schema) => nisbac::back::rust::generate(&schema, CodeGenKind::Encode, Config {}).into(),
        Err(e) => proc_macro_error::abort!(path.span(), e),
    }
}

#[proc_macro_error]
#[proc_macro]
pub fn generate_decode(input: TokenStream) -> TokenStream {
    let path: syn::LitStr = match syn::parse(input) {
        Ok(x) => x,
        Err(e) => proc_macro_error::abort!(e.span(), e),
    };
    match get_schema(path.value()) {
        Ok(schema) => nisbac::back::rust::generate(&schema, CodeGenKind::Decode, Config {}).into(),
        Err(e) => proc_macro_error::abort!(path.span(), e),
    }
}
