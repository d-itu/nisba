use crate::Ident;

use super::*;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Number {
    Value(u64),
    Overflow,
}

#[derive(Debug, PartialEq)]
pub struct Signed(pub Number);

#[derive(Debug, PartialEq)]
pub struct Unsigned(pub Number);

#[derive(Debug, PartialEq)]
pub struct Varint<T>(pub Spanned<T>);

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SequenceKind {
    Vector,
    Stream,
}

#[derive(Debug, PartialEq)]
pub enum LenType {
    Fixed(Unsigned),
    Varint(Varint<Unsigned>),
}

#[derive(Debug, PartialEq)]
pub struct SequenceLike {
    pub kind: SequenceKind,
    pub len_ty: Spanned<LenType>,
    pub elem_ty: Spanned<Type>,
}

#[derive(Debug, PartialEq)]
pub enum Type {
    Sequence(Box<SequenceLike>),
    VarintSigned(Varint<Signed>),
    VarintUnsigned(Varint<Unsigned>),
    Ident(Ident),
    Builtin(Builtin),
}

#[derive(Debug, PartialEq)]
pub enum Builtin {
    Void,
    Unsigned(Unsigned),
    Signed(Signed),
}

#[derive(Debug, PartialEq)]
pub struct Primitive {
    pub ty: Spanned<Builtin>,
}

#[derive(Debug, PartialEq)]
pub enum StructKind {
    Struct,
    Packed,
}

#[derive(Debug, PartialEq)]
pub struct Member {
    pub name: Spanned<Ident>,
    pub ty: Spanned<Type>,
}

#[derive(Debug, PartialEq)]
pub enum IndexedStructKind {
    Enum,
    Dict,
}

#[derive(Debug, PartialEq)]
pub struct IndexedMember {
    pub name: Spanned<Ident>,
    pub ty: Option<Spanned<Type>>,
    pub index: Option<Spanned<Number>>,
}

#[derive(Debug, PartialEq)]
pub enum Definition {
    Extern(Option<Spanned<Builtin>>),
    StructLike {
        kind: StructKind,
        members: Vec<Spanned<Member>>,
    },
    IndexedStructLike {
        kind: IndexedStructKind,
        index_ty: Spanned<Unsigned>,
        members: Vec<Spanned<IndexedMember>>,
    },
}

#[derive(Debug, PartialEq)]
pub struct NamedDefinition {
    pub name: Spanned<Ident>,
    pub def: Definition,
}

#[derive(Debug, PartialEq)]
pub struct Document {
    pub definitions: Vec<Spanned<NamedDefinition>>,
}
