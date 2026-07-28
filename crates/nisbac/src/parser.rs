use smol_str::{SmolStr, SmolStrBuilder};
use thiserror::Error;

use ast::*;

pub type ParseError = peg::error::ParseError<peg::str::LineCol>;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Span {
    start: u32,
    end: u32,
}

impl Span {
    pub fn with<T>(self, item: T) -> Spanned<T> {
        Spanned { item, span: self }
    }
    pub fn spanned<T>(self) -> impl FnOnce(T) -> Spanned<T> {
        #[inline]
        move |item| self.with(item)
    }
    pub fn show(self, source: &str) -> &str {
        &source[self.start as usize..self.end as usize]
    }
}

#[derive(Error, Debug, Clone, Copy, PartialEq)]
#[error("{item}")]
pub struct Spanned<T> {
    pub item: T,
    pub span: Span,
}

impl<T> Spanned<T> {
    pub fn map<U, F: FnOnce(&T) -> U>(&self, f: F) -> Spanned<U> {
        Spanned {
            span: self.span,
            item: f(&self.item),
        }
    }
    pub fn replace<U>(&self, item: U) -> Spanned<U> {
        Spanned {
            span: self.span,
            item,
        }
    }
}

pub trait IntoSpanned: Sized {
    fn into_spanned(self, span: Span) -> Spanned<Self> {
        Spanned { item: self, span }
    }
}

impl<T> IntoSpanned for T {}

impl ast::Number {
    fn new(value: u8) -> Self {
        Self::Value(value as _)
    }
    fn update(&mut self, mul: u8, add: u8) {
        match self {
            ast::Number::Value(value) => {
                match value
                    .checked_mul(mul as _)
                    .and_then(|x| x.checked_add(add as _))
                {
                    Some(x) => *value = x,
                    None => *self = Self::Overflow,
                }
            }
            ast::Number::Overflow => {}
        }
    }
}

peg::parser! {
  grammar nisba() for str {
    inject span(_input, start, end) -> Span {
        Span {
            start: start as _,
            end: end as _,
        }
    }

    pub rule doc() -> Document
        = _ definitions:definition()* { Document { definitions } }

    rule definition() -> Spanned<NamedDefinition>
        = x:extern() _ { span.with(x) }
        / x:struct_like() _ { span.with(x) }
        / x:indexed_struct_like() _ { span.with(x) }

    rule extern_body() -> Option<Builtin>
        = ['('] _ builtin:builtin() _ [')'] _ { Some(builtin) }
        / ws() { None }
    rule extern() -> NamedDefinition
        = "@extern" _ alias:extern_body() _ name:valid_name() {
            NamedDefinition {
                name: span.with(name),
                def: Definition::Extern(alias.map(span.spanned())),
            }
        }

    rule member() -> Spanned<Member>
        = name:identifier() _ [':'] _ ty:ty() _ {
            span.with(Member {
                name: span.with(name),
                ty: span.with(ty),
            })
        }
    rule struct_like() -> NamedDefinition
        = kind:struct_kind() _ name:valid_name() _ ['{'] _ members:member()* ['}'] {
            NamedDefinition {
                name: span.with(name),
                def: Definition::StructLike { kind, members },
            }
        }

    rule member_type() -> Type = [':'] _ ty:ty() { ty }
    rule member_index() -> Number = ['='] _ index:number() { index }
    rule indexed_member() -> Spanned<IndexedMember>
        = name:identifier() _ ty:member_type()? _ index:member_index()? _ {
            span.with(IndexedMember {
                name: span.with(name),
                ty: ty.map(span.spanned()),
                index: index.map(span.spanned()),
            })
        }
    rule indexed_struct_like() -> NamedDefinition
        = kind:indexed_struct_kind() _ ['('] _ index_ty:unsigned() _ [')'] _ name:valid_name() _ ['{'] _ members:indexed_member()* ['}']
        {
            NamedDefinition {
                name: span.with(name),
                def:Definition::IndexedStructLike {
                    kind,
                    index_ty: span.with(index_ty),
                    members,
                },
            }
        }

    rule ty() -> Type
        = x:sequence_like() { Type::Sequence(Box::new(x)) }
        / x:varint_signed() { Type::VarintSigned(x) }
        / x:varint_unsigned() { Type::VarintUnsigned(x) }
        / x:valid_name() { Type::Ident(x) }
        / x:builtin() { Type::Builtin(x) }

    rule sequence_like() -> SequenceLike
        = kind:sequence_kind() _ ['('] _ len_ty:len_type() _ elem_ty:ty() _ [')'] {
            SequenceLike {
                kind,
                len_ty: span.with(len_ty),
                elem_ty: span.with(elem_ty),
            }
        }

    rule len_type() -> LenType
        = x:unsigned() { LenType::Fixed(x) }
        / x:varint_unsigned() { LenType::Variant(x) }

    rule varint_unsigned() -> Varint<Unsigned>
        = "@varint" _ ['('] _ x:unsigned() _ [')'] { Varint(span.with(x)) }
    rule varint_signed() -> Varint<Signed>
        = "@varint" _ ['('] _ x:signed() _ [')'] { Varint(span.with(x)) }

    rule valid_name() -> SmolStr
        = !(builtin() / ['{' | '@' | ')' | '}' | '='] / ws() / eof()) x:identifier() { x }

    rule builtin() -> Builtin
        = "void" { Builtin::Void }
        / x:signed() { Builtin::Signed(x) }
        / x:unsigned() { Builtin::Unsigned(x) }

    rule unsigned() -> Unsigned = "u" x:packed_dec() { Unsigned(x) }
    rule signed() -> Signed = "i" x:packed_dec() { Signed(x) }

    rule packed_dec() -> Number
        = ['0'] { Number::new(0) }
        / x:['1'..='9'] xs:$(['0'..='9' | '_']*) {
            let mut value = Number::new(x as u8 - b'0');
            for x in xs.as_bytes() {
                if let b'0'..=b'9' = x { value.update(10, x - b'0') }
            }
            value
        }

    rule number() -> Number
        = ['0'] ['b' | 'B'] ['_']* x:['0' | '1'] xs:$(['0' | '1' | '_']*) {
            let mut value = Number::new(x as u8 - b'0');
            for x in xs.as_bytes() {
                if let b'0' | b'1' = x { value.update(2, x - b'0') }
            }
            value
        }
        / ['0'] ['x' | 'X'] ['_']* [x if let Some(x) = x.to_digit(16)] xs:$(['0'..='9' | 'a' ..='f' | 'A' ..='F' | '_']*) {
            let mut value = Number::new(x as _);
            for x in xs.as_bytes() {
                match x {
                    b'0'..=b'9' => value.update(16, x - b'0'),
                    b'a'..=b'f' => value.update(16, x - b'a' + 10),
                    b'A'..=b'F' => value.update(16, x - b'A' + 10),
                    _ => {}
                }
            }
            value
        }
        / x:['0'..='9'] xs:$(['0'..='9' | '_']*) {
            let mut value = Number::new(x as u8 - b'0');
            for x in xs.as_bytes() {
                if let b'0'..=b'9' = x { value.update(10, x - b'0') }
            }
            value
        }

    rule identifier() -> SmolStr
        = x: ['a'..='z' | 'A'..='Z'] xs: $(['a'..='z' | 'A'..='Z' | '0'..='9' | '_']*) {
            let mut builder = SmolStrBuilder::new();
            builder.push(x);
            builder.push_str(xs);
            builder.finish()
        }

    rule sequence_kind() -> SequenceKind
        = "@vector" { SequenceKind::Vector }
        / "@stream" { SequenceKind::Stream }
    rule struct_kind() -> StructKind
        = "@struct" { StructKind::Struct }
        / "@packed" { StructKind::Packed }
    rule indexed_struct_kind() -> IndexedStructKind
        = "@enum" { IndexedStructKind::Enum }
        / "@dict" { IndexedStructKind::Dict }

    rule _() = ws()*
    rule ws() = [' ' | '\r' | '\n' | '\t'] / comment()

    rule comment() = "//" (!(['\n'] / eof()) [_])+

    rule eof() = ![_]
  }
}

pub fn parse(source: &str) -> Result<ast::Document, ParseError> {
    nisba::doc(source)
}

pub mod ast;
