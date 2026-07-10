use std::{
    fs,
    io::{self, Write as _},
    path::PathBuf,
};

use clap::{Parser, ValueEnum};
use nisbac::{Error, back, parser, schema};

#[derive(ValueEnum, Debug, Clone, Copy)]
enum Lang {
    Rust,
}

#[derive(ValueEnum, Debug, Clone, Copy)]
enum Kind {
    Encode,
    Decode,
}

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    kind: Option<Kind>,

    /// Path to nisba source file. Read from stdin if omitted
    #[arg(long, short)]
    input: Option<PathBuf>,
    /// Target language
    #[arg(long, short)]
    lang: Option<Lang>,
    /// Output file path. Print to stdout if omitted
    #[arg(long, short)]
    output: Option<PathBuf>,
}

fn main() {
    let args = Args::parse();
    let src = if let Some(path) = &args.input {
        fs::read_to_string(path).unwrap()
    } else {
        io::read_to_string(io::stdin()).unwrap()
    };

    let src = &src;

    let doc = parser::parse(src)
        .map_err(|e| Error {
            kind: e.into(),
            src,
        })
        .unwrap();
    let schema = schema::resolve(&doc)
        .map_err(|e| Error {
            kind: e.into(),
            src,
        })
        .unwrap();
    let validated = schema::validate(schema)
        .map_err(|e| Error {
            kind: e.into(),
            src,
        })
        .unwrap();

    let (lang, kind) = if let (Some(lang), Some(kind)) = (args.lang, args.kind) {
        (
            lang,
            match kind {
                Kind::Encode => back::CodeGenKind::Encode,
                Kind::Decode => back::CodeGenKind::Decode,
            },
        )
    } else {
        return;
    };

    let output = match lang {
        Lang::Rust => {
            let tokens = back::rust::generate(&validated, kind, Default::default());
            let file = syn::parse2(tokens).unwrap();
            prettyplease::unparse(&file)
        }
    };

    if let Some(path) = args.output {
        fs::write(path, output).unwrap();
    } else {
        io::stdout().write_all(output.as_bytes()).unwrap();
    }
}
