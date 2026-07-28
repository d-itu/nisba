use std::fmt::{self, Debug, Display, Formatter};

use thiserror::Error;

pub(crate) type Ident = smol_str::SmolStr;

pub mod back;
pub mod parser;
pub mod schema;

pub use back::CodeGenKind;
use parser::ParseError;
use schema::{resolver, validator};

#[derive(Error, Debug)]
pub enum Kind {
    #[error(transparent)]
    Parse(#[from] ParseError),
    #[error(transparent)]
    Resolve(#[from] resolver::Error),
    #[error(transparent)]
    Validate(#[from] validator::Error),
}
#[derive(Error)]
pub struct Error<T> {
    pub kind: Kind,
    pub src: T,
}

impl<T: AsRef<str>> Debug for Error<T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{self}")
    }
}

impl<T: AsRef<str>> Display for Error<T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match &self.kind {
            Kind::Parse(e) => write!(f, "{e}"),
            Kind::Resolve(e) => match e {
                resolver::Error::Spanned(e) => {
                    write!(f, r#"{}: "{}""#, e.item, e.span.show(self.src.as_ref()))
                }
                resolver::Error::UnknownTypeName(_) => write!(f, "unknown typename"),
            },
            Kind::Validate(e) => write!(f, "{e}"),
        }
    }
}

type CodeGenResult<T> = Result<String, Error<T>>;

pub fn generate(src: &str, kind: CodeGenKind) -> CodeGenResult<&str> {
    let doc = parser::parse(src).map_err(|e| Error {
        kind: e.into(),
        src,
    })?;
    let schema = schema::resolve(&doc).map_err(|e| Error {
        kind: e.into(),
        src,
    })?;
    let validated = schema::validate(schema).map_err(|e| Error {
        kind: e.into(),
        src,
    })?;
    let tokens = back::rust::generate(&validated, kind, Default::default());
    let file = syn::parse2(tokens).unwrap();
    Ok(prettyplease::unparse(&file))
}
