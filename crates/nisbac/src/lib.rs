use thiserror::Error;

#[derive(Error, Debug)]
pub enum Error {
    #[error(transparent)]
    ParseError(#[from] pest::error::Error<ast::Rule>),
    #[error("Multiple definitions for identifier '{0}'")]
    MultipleDefinitions(String),
    #[error("Duplicate member name '{0}'")]
    DuplicateMemberName(String),
    #[error("Tag assigned more than once")]
    TagAssignedMoreThanOnce,
    #[error("Invalid tag bits {0}")]
    InvalidTagBits(u16),
    #[error("Tag value out of range")]
    TagValueOutOfRange,
    #[error("Element not byte aligned")]
    ElementNotByteAligned,
    #[error("Variable size not supported")]
    VariableSize,
    #[error("Unresolved reference")]
    UnresolvedReference,
}

pub mod ast;
pub mod backend;
pub mod schema;
