use crate::{
    Error, Ident,
    schema::{self, Handle},
};

#[derive(Debug)]
pub enum TypeDef {
    Unresolved { name: Ident, value: TypeDefValue },
    Resolved(Handle),
}

#[derive(Debug)]
pub enum TypeDefValue {
    Primitive(Primitive),
    Tuple {
        kind: TupleKind,
        members: Vec<Member>,
    },
    Union {
        kind: UnionKind,
        discriminant_bit_width: Unsigned,
        members: Vec<TaggedMember>,
    },
}

#[derive(Debug)]
pub enum Primitive {
    Signed(Signed),
    Unsigned(Unsigned),
    Void,
}

#[derive(Debug)]
pub enum UnionKind {
    Enum,
    Dict,
}

#[derive(Debug)]
pub enum TupleKind {
    Packed,
    Struct,
}

#[derive(Debug)]
pub struct Member {
    pub name: Ident,
    pub ty: Type,
}

#[derive(Debug)]
pub struct TaggedMember {
    pub name: Ident,
    pub ty: Option<Type>,
    pub discriminant: Option<u64>,
}

#[derive(Debug)]
pub enum UnresolvedType {
    Ident(Ident),
    Signed(Signed),
    Unsigned(Unsigned),
    Void,
    VarintSigned(VarintSigned),
    VarintUnsigned(VarintUnsigned),
    Array {
        kind: ArrayKind,
        len_type: LenType,
        item_type: Box<Type>,
    },
}

#[derive(Debug)]
pub enum Type {
    Unresolved(UnresolvedType),
    Resolved(schema::Type),
}

impl Type {
    pub fn resolved(&self) -> Option<schema::Type> {
        if let &Self::Resolved(x) = self {
            Some(x)
        } else {
            None
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum LenType {
    Unsigned(Unsigned),
    VarintUnsigned(VarintUnsigned),
}

#[derive(Debug)]
pub enum ArrayKind {
    Vector,
    Stream,
}

#[derive(Debug, Clone, Copy)]
pub struct Signed(pub u16);
#[derive(Debug, Clone, Copy)]
pub struct Unsigned(pub u16);
#[derive(Debug, Clone, Copy)]
pub struct VarintSigned(pub u16);
#[derive(Debug, Clone, Copy)]
pub struct VarintUnsigned(pub u16);

use ahash::AHashSet;
use pest::{
    Parser as _,
    iterators::{Pair, Pairs},
};

mod parser {
    #[derive(pest_derive::Parser)]
    #[grammar = "nisba.pest"]
    pub struct Parser;
}

pub use parser::*;

fn parse_dec(pair: Pair<Rule>) -> u64 {
    debug_assert_eq!(pair.as_rule(), Rule::Dec);
    pair.as_str().parse().unwrap()
}

fn parse_unsigned(pair: Pair<Rule>) -> Unsigned {
    debug_assert_eq!(pair.as_rule(), Rule::Unsigned);
    Unsigned(parse_dec(pair.into_inner().next().unwrap()) as _)
}

fn parse_signed(pair: Pair<Rule>) -> Signed {
    debug_assert_eq!(pair.as_rule(), Rule::Signed);
    Signed(parse_dec(pair.into_inner().next().unwrap()) as _)
}

fn parse_varint_unsigned(pair: Pair<Rule>) -> VarintUnsigned {
    debug_assert_eq!(pair.as_rule(), Rule::VarintUnsigned);
    VarintUnsigned(parse_unsigned(pair.into_inner().next().unwrap()).0)
}

fn parse_varint_signed(pair: Pair<Rule>) -> VarintSigned {
    debug_assert_eq!(pair.as_rule(), Rule::VarintSigned);
    VarintSigned(parse_signed(pair.into_inner().next().unwrap()).0)
}

fn parse_type(pair: Pair<Rule>) -> Type {
    debug_assert_eq!(pair.as_rule(), Rule::Type);
    let ty = pair.into_inner().next().unwrap();
    let ty = match ty.as_rule() {
        Rule::Array => {
            let mut inner = ty.into_inner();
            let kind = match inner.next().unwrap().as_rule() {
                Rule::Vector => ArrayKind::Vector,
                Rule::Stream => ArrayKind::Stream,
                _ => unreachable!(),
            };
            let pair = inner.next().unwrap();
            let len_type = match pair.as_rule() {
                Rule::VarintUnsigned => LenType::VarintUnsigned(parse_varint_unsigned(pair)),
                Rule::Unsigned => LenType::Unsigned(parse_unsigned(pair)),
                _ => unreachable!(),
            };
            let item_type = Box::new(parse_type(inner.next().unwrap()));
            UnresolvedType::Array {
                kind,
                len_type,
                item_type,
            }
        }
        Rule::VarintSigned => UnresolvedType::VarintSigned(parse_varint_signed(ty)),
        Rule::VarintUnsigned => UnresolvedType::VarintUnsigned(parse_varint_unsigned(ty)),
        Rule::Signed => UnresolvedType::Signed(parse_signed(ty)),
        Rule::Unsigned => UnresolvedType::Unsigned(parse_unsigned(ty)),
        Rule::Void => UnresolvedType::Void,
        Rule::Ident => UnresolvedType::Ident(ty.as_str().into()),
        _ => unreachable!(),
    };
    Type::Unresolved(ty)
}

fn parse_number(pair: Pair<Rule>) -> u64 {
    debug_assert_eq!(pair.as_rule(), Rule::Number);
    let pair = pair.into_inner().next().unwrap();
    match pair.as_rule() {
        Rule::Dec => pair.as_str().parse().unwrap(),
        Rule::Hex => u64::from_str_radix(pair.as_str().strip_prefix("0x").unwrap(), 16).unwrap(),
        Rule::Bin => u64::from_str_radix(pair.as_str().strip_prefix("0b").unwrap(), 2).unwrap(),
        _ => unreachable!(),
    }
}

fn parse_members(pairs: Pairs<Rule>) -> Result<Vec<Member>, Error> {
    let mut result = vec![];
    let mut used_names = AHashSet::new();
    for pair in pairs {
        debug_assert_eq!(pair.as_rule(), Rule::Member);
        let mut inner = pair.into_inner();
        let name: Ident = inner.next().unwrap().as_str().into();
        if !used_names.insert(name.clone()) {
            Err(Error::DuplicateMemberName(name.clone()))?
        }
        let ty = parse_type(inner.next().unwrap());
        result.push(Member { name, ty });
    }
    Ok(result)
}

fn parse_tagged_members(pairs: Pairs<Rule>) -> Result<Vec<TaggedMember>, Error> {
    let mut result = vec![];
    let mut used_names = AHashSet::new();
    for pair in pairs {
        let mut inner = pair.into_inner();
        let ident = inner.next().unwrap();
        debug_assert_eq!(ident.as_rule(), Rule::Ident);
        let mut ty = None;
        let mut discriminant = None;
        for pair in inner {
            match pair.as_rule() {
                Rule::Type => ty = Some(parse_type(pair)),
                Rule::Number => discriminant = Some(parse_number(pair)),
                _ => unreachable!(),
            }
        }
        let name: Ident = ident.as_str().into();
        if !used_names.insert(name.clone()) {
            Err(Error::DuplicateMemberName(name.clone()))?
        }
        result.push(TaggedMember {
            name,
            ty,
            discriminant,
        });
    }
    Ok(result)
}

pub fn parse(input: &str) -> Result<Vec<TypeDef>, Error> {
    let doc = Parser::parse(Rule::Doc, input)?.next().unwrap();
    let mut result = vec![];
    for typedef in doc.into_inner().take_while(|x| x.as_rule() != Rule::EOI) {
        let rule = typedef.as_rule();
        let mut inner = typedef.into_inner();
        let typedef = match rule {
            Rule::Primitive => {
                let ident = inner.next().unwrap();
                debug_assert_eq!(ident.as_rule(), Rule::Ident);
                let ty = inner.next().unwrap();
                TypeDef::Unresolved {
                    name: ident.as_str().into(),
                    value: TypeDefValue::Primitive(match ty.as_rule() {
                        Rule::Signed => Primitive::Signed(parse_signed(ty)),
                        Rule::Unsigned => Primitive::Unsigned(parse_unsigned(ty)),
                        Rule::Void => Primitive::Void,
                        _ => unreachable!(),
                    }),
                }
            }
            Rule::Tuple => {
                let kind = match inner.next().unwrap().as_rule() {
                    Rule::Packed => TupleKind::Packed,
                    Rule::Struct => TupleKind::Struct,
                    _ => unreachable!(),
                };
                let ident = inner.next().unwrap();
                debug_assert_eq!(ident.as_rule(), Rule::Ident);
                let members = parse_members(inner)?;
                TypeDef::Unresolved {
                    name: ident.as_str().into(),
                    value: TypeDefValue::Tuple { kind, members },
                }
            }
            Rule::Union => {
                let kind = match inner.next().unwrap().as_rule() {
                    Rule::Enum => UnionKind::Enum,
                    Rule::Dict => UnionKind::Dict,
                    _ => unreachable!(),
                };
                let width = parse_unsigned(inner.next().unwrap());
                let ident = inner.next().unwrap();
                debug_assert_eq!(ident.as_rule(), Rule::Ident);
                let members = parse_tagged_members(inner)?;
                TypeDef::Unresolved {
                    name: ident.as_str().into(),
                    value: TypeDefValue::Union {
                        kind,
                        discriminant_bit_width: width,
                        members,
                    },
                }
            }
            _ => unreachable!(),
        };
        result.push(typedef);
    }
    Ok(result)
}
