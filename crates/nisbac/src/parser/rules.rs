use std::cmp::{Ord, Ordering};

use either::Either;
use smol_str::SmolStrBuilder;

use crate::Ident;

use super::*;

impl Parser<'_> {
    fn ws(&mut self) {
        while let Ok(_) = self.match_rule(WHITE_SPACE) {}
    }
    fn ws_plus(&mut self) -> ParseResult<()> {
        self.match_rule(WHITE_SPACE)?;
        self.ws();
        Ok(())
    }
}

#[allow(unused_macros)]
macro_rules! choice_type {
    ($x:ty, $y:ty $(,)?) => {
        Choice<$x, $y>
    };
    ($x:ty $(,$xs:ty)* $(,)?) => {
        Choice<$x, choice_type!($($xs,)*)>
    };
}

macro_rules! choice {
    ($x:expr, $y:expr $(,)?) => {
        Choice($x, $y)
    };
    ($x:expr $(,$xs:expr)* $(,)?) => {
        Choice($x, choice!($($xs,)*))
    };
}

#[allow(unused_macros)]
macro_rules! variant_type {
    ($x:ty, $y:ty $(,)?) => {
        Variant<$x, $y>
    };
    ($x:ty $(,$xs:ty)* $(,)?) => {
        Variant<$x, variant_type!($($xs,)*)>
    };
}

macro_rules! variant {
    ($x:expr, $y:expr $(,)?) => {
        variant($x, $y)
    };
    ($x:expr $(,$xs:expr)* $(,)?) => {
        variant($x, variant!($($xs,)*))
    };
}

struct End;
impl Rule for End {
    type Item = ();
    const EXPECTED: Tree = Tree::End;

    fn parse(self, parser: &mut Parser) -> ParseResult<Self::Item> {
        if parser.cursor.next().is_none() {
            return Ok(());
        }
        Self::unmatched()
    }
}

struct Char<const VALUE: char>;
impl<const VALUE: char> Rule for Char<VALUE> {
    type Item = char;

    const EXPECTED: Tree = Tree::Char(VALUE);

    fn parse(self, parser: &mut Parser) -> ParseResult<Self::Item> {
        match parser.cursor.next() {
            Some(x) if x == VALUE => Ok(x),
            _ => Self::unmatched(),
        }
    }
}

#[allow(unused)]
struct Range<const BEGIN: char, const END: char>;
impl<const BEGIN: char, const END: char> Rule for Range<BEGIN, END> {
    const EXPECTED: Tree = Tree::Range(BEGIN..END);
    type Item = char;

    fn parse(self, parser: &mut Parser) -> ParseResult<Self::Item> {
        match parser.cursor.next() {
            Some(x) if (BEGIN..END).contains(&x) => Ok(x),
            _ => Self::unmatched()?,
        }
    }
}

struct RangeInclusive<const BEGIN: char, const END: char>;
impl<const BEGIN: char, const END: char> Rule for RangeInclusive<BEGIN, END> {
    const EXPECTED: Tree = Tree::RangeInclusive(BEGIN..=END);
    type Item = char;

    fn parse(self, parser: &mut Parser) -> ParseResult<Self::Item> {
        match parser.cursor.next() {
            Some(x) if (BEGIN..=END).contains(&x) => Ok(x),
            _ => Self::unmatched()?,
        }
    }
}

type Variant<X, Y> = Map<Choice<X, Y>, EitherIntoInner>;
const fn variant<T, X: Rule<Item = T>, Y: Rule<Item = T>>(x: X, y: Y) -> Variant<X, Y> {
    map(Choice(x, y), EitherIntoInner)
}

struct Choice<X, Y>(X, Y);
impl<X: Rule, Y: Rule> Rule for Choice<X, Y> {
    type Item = Either<X::Item, Y::Item>;
    const EXPECTED: Tree = Tree::Choice(&X::EXPECTED, &Y::EXPECTED);

    fn parse(self, parser: &mut Parser) -> ParseResult<Self::Item> {
        let start = parser.cursor.byte_offset;

        let l_err = match parser.parse(self.0) {
            Ok(x) => return Ok(Either::Left(x)),
            Err(e) => e,
        };
        let l_len = parser.cursor.byte_offset;

        parser.cursor.byte_offset = start;
        let r_err = match parser.parse(self.1) {
            Ok(x) => return Ok(Either::Right(x)),
            Err(e) => e,
        };
        let r_len = parser.cursor.byte_offset;

        parser.cursor.byte_offset = Ord::max(l_len, r_len);
        Err(match Ord::cmp(&l_len, &r_len) {
            Ordering::Less => r_err,
            Ordering::Greater => l_err,
            Ordering::Equal => Unmatched {
                expected: Self::EXPECTED,
            },
        })
    }
}

impl<X: Rule, Y: Rule> Rule for (X, Y) {
    type Item = (X::Item, Y::Item);
    const EXPECTED: Tree = X::EXPECTED;

    fn parse(self, parser: &mut Parser) -> ParseResult<Self::Item> {
        let x = parser.parse(self.0)?;
        let y = parser.parse(self.1)?;
        Ok((x, y))
    }
}

struct Not<P>(P);
impl<P: Rule> Rule for Not<P> {
    type Item = ();
    const EXPECTED: Tree = Tree::Not(&P::EXPECTED);

    fn parse(self, parser: &mut Parser) -> ParseResult<Self::Item> {
        let start = parser.cursor.byte_offset;
        if parser.parse(self.0).is_ok() {
            return Self::unmatched();
        }
        parser.cursor.byte_offset = start;
        Ok(())
    }
}

type AsciiAlpha = Variant<RangeInclusive<'a', 'z'>, RangeInclusive<'A', 'Z'>>;
const ASCII_ALPHA: AsciiAlpha = variant(RangeInclusive::<'a', 'z'>, RangeInclusive::<'A', 'Z'>);

type AsciiAlphanumeric = Variant<AsciiAlpha, RangeInclusive<'0', '9'>>;
const ASCII_ALPHANUMERIC: AsciiAlphanumeric = variant(ASCII_ALPHA, RangeInclusive::<'0', '9'>);

/// ASCII_ALPHA (ASCII_ALPHANUMERIC | "_")*
struct Identifier;
impl Rule for Identifier {
    type Item = Ident;
    const EXPECTED: Tree = Tree::Rule("identifier");

    fn parse(self, parser: &mut Parser) -> ParseResult<Self::Item> {
        let mut builder = SmolStrBuilder::new();
        builder.push(ASCII_ALPHA.parse(parser)?);
        while let Ok(x) = parser.match_rule(variant!(ASCII_ALPHANUMERIC, Char::<'_'>)) {
            builder.push(x)
        }
        Ok(builder.finish())
    }
}

/// [ \r\n\t]
struct WhitespaceChar;
impl Rule for WhitespaceChar {
    type Item = char;
    const EXPECTED: Tree = Tree::Rule("whitespace char");

    fn parse(self, parser: &mut Parser) -> ParseResult<Self::Item> {
        parser.match_set(" \r\n\t")
    }
}

/// "//" (!("\n" | END) .)*
struct Comment;
impl Rule for Comment {
    type Item = ();
    const EXPECTED: Tree = Tree::Rule("comment");

    fn parse(self, parser: &mut Parser) -> ParseResult<Self::Item> {
        parser.match_str("//")?;
        loop {
            if let Some('\n') | None = parser.cursor.peek() {
                break;
            }
            parser.cursor.next();
        }
        Ok(())
    }
}

#[doc(alias = "ws")]
type WhiteSpace = Choice<WhitespaceChar, Comment>;
const WHITE_SPACE: WhiteSpace = Choice(WhitespaceChar, Comment);

impl ast::Number {
    const ZERO: Self = Self::Value(0);
    fn update(&mut self, mul: u64, add: u64) {
        match self {
            ast::Number::Value(value) => {
                match value.checked_mul(mul) {
                    Some(x) => *value = x,
                    None => return *self = Self::Overflow,
                };
                match value.checked_add(add) {
                    Some(x) => *value = x,
                    None => return *self = Self::Overflow,
                };
            }
            ast::Number::Overflow => {}
        }
    }
}

/// "0" [bB] [_]* [01] [01_]*
struct Bin;
impl Rule for Bin {
    type Item = ast::Number;
    const EXPECTED: Tree = Tree::Rule("bin");

    fn parse(self, parser: &mut Parser) -> ParseResult<Self::Item> {
        parser.match_char('0')?;
        parser.match_set("bB")?;

        while let Ok(_) = parser.match_char('_') {}

        let mut result = ast::Number::ZERO;

        match parser.cursor.peek() {
            Some('0') => result.update(2, 0),
            Some('1') => result.update(2, 1),
            _ => Err(Tree::RangeInclusive('0'..='1'))?,
        }
        parser.cursor.next();

        loop {
            match parser.cursor.peek() {
                Some('0') => result.update(2, 0),
                Some('1') => result.update(2, 1),
                Some('_') => {}
                _ => break Ok(result),
            }
            parser.cursor.next();
        }
    }
}

/// "0" [xX] [_]* [0-9a-fA-F] [0-9a-fA-F_]*
struct Hex;
impl Rule for Hex {
    type Item = ast::Number;
    const EXPECTED: Tree = Tree::Rule("hex");

    fn parse(self, parser: &mut Parser) -> ParseResult<Self::Item> {
        parser.match_char('0')?;
        parser.match_set("xX")?;

        while let Ok(_) = parser.match_char('_') {}

        let mut result = ast::Number::ZERO;

        match parser.cursor.peek() {
            Some(x @ '0'..='9') => result.update(16, x as u64 - '0' as u64),
            Some(x @ 'a'..='f') => result.update(16, x as u64 - 'a' as u64 + 10),
            Some(x @ 'A'..='F') => result.update(16, x as u64 - 'A' as u64 + 10),
            _ => Err(Tree::Rule("[0-9a-fA-F]"))?,
        }
        parser.cursor.next();

        loop {
            match parser.cursor.peek() {
                Some(x @ '0'..='9') => result.update(16, x as u64 - '0' as u64),
                Some(x @ 'a'..='f') => result.update(16, x as u64 - 'a' as u64 + 10),
                Some(x @ 'A'..='F') => result.update(16, x as u64 - 'A' as u64 + 10),
                Some('_') => {}
                _ => break Ok(result),
            }
            parser.cursor.next();
        }
    }
}

/// [0-9] [0-9_]*
struct Dec;
impl Rule for Dec {
    type Item = ast::Number;
    const EXPECTED: Tree = Tree::Rule("dec");

    fn parse(self, parser: &mut Parser) -> ParseResult<Self::Item> {
        let mut result = ast::Number::ZERO;
        match parser.cursor.peek() {
            Some(x @ '0'..='9') => result.update(10, x as u64 - '0' as u64),
            _ => Err(Tree::RangeInclusive('0'..='9'))?,
        }
        parser.cursor.next();

        loop {
            match parser.cursor.peek() {
                Some(x @ '0'..='9') => result.update(10, x as u64 - '0' as u64),
                Some('_') => {}
                _ => break Ok(result),
            }
            parser.cursor.next();
        }
    }
}

/// "0" | [1-9] [0-9]*
struct PackedDec;
impl Rule for PackedDec {
    type Item = ast::Number;
    const EXPECTED: Tree = Tree::Rule("packed dec");

    fn parse(self, parser: &mut Parser) -> ParseResult<Self::Item> {
        let mut result = ast::Number::ZERO;
        match parser.cursor.peek() {
            Some(x @ '1'..='9') => result.update(10, x as u64 - '0' as u64),
            Some('0') => {
                parser.cursor.next();
                return Ok(result);
            }
            _ => Err(Tree::RangeInclusive('0'..='9'))?,
        }
        parser.cursor.next();

        loop {
            match parser.cursor.peek() {
                Some(x @ '0'..='9') => result.update(10, x as u64 - '0' as u64),
                _ => break Ok(result),
            }
            parser.cursor.next();
        }
    }
}

/// bin | hex | dec
struct Number;
impl Rule for Number {
    type Item = ast::Number;
    const EXPECTED: Tree = Tree::Rule("number");

    fn parse(self, parser: &mut Parser) -> ParseResult<Self::Item> {
        parser.parse(variant!(Bin, Hex, Dec))
    }
}

#[test]
fn test_ident() {
    let result = Identifier
        .parse(&mut Parser::new("i0_0X"))
        .inspect_err(|e| eprintln!("{}", e))
        .unwrap();
    assert_eq!(result, "i0_0X")
}

#[test]
fn test_integer() {
    let bin = Bin.parse(&mut Parser::new("0b_1100_")).unwrap();
    assert_eq!(bin, ast::Number::Value(0b1100));
    let hex = Hex.parse(&mut Parser::new("0x1Ff100aaEabbbbb")).unwrap();
    assert_eq!(hex, ast::Number::Value(0x1Ff100aaEabbbbb));
    let dec = Dec.parse(&mut Parser::new("000")).unwrap();
    assert_eq!(dec, ast::Number::Value(000));

    let x = Number
        .parse(&mut Parser::new("0xff_ff_ff_ff_ff_ff_ff_ff_f"))
        .unwrap();
    assert_eq!(x, ast::Number::Overflow);

    assert!(Number.parse(&mut Parser::new("")).is_err());
}

/// "u" packed-dec
struct Unsigned;
impl Rule for Unsigned {
    type Item = ast::Unsigned;
    const EXPECTED: Tree = Tree::Rule("unsigned");

    fn parse(self, parser: &mut Parser) -> ParseResult<Self::Item> {
        parser.match_char('u')?;
        parser.match_rule(PackedDec).map(ast::Unsigned)
    }
}

/// "i" packed-dec
struct Signed;
impl Rule for Signed {
    type Item = ast::Signed;
    const EXPECTED: Tree = Tree::Rule("unsigned");

    fn parse(self, parser: &mut Parser) -> ParseResult<Self::Item> {
        parser.match_char('i')?;
        parser.match_rule(PackedDec).map(ast::Signed)
    }
}

/// "@varint" ws* "(" ws* unsigned ws* ")"
/// "@varint" ws* "(" ws* signed ws* ")"
struct Varint<T>(T);
impl<T: Rule> Rule for Varint<T> {
    type Item = ast::Varint<T::Item>;
    const EXPECTED: Tree = Tree::Rule("varint");

    fn parse(self, parser: &mut Parser) -> ParseResult<Self::Item> {
        let start = parser.cursor.byte_offset as _;
        parser.match_str("@varint")?;
        parser.ws();
        parser.match_char('(')?;
        parser.ws();
        let item = parser.match_rule(self.0)?;
        parser.ws();
        parser.match_char(')')?;
        let end = parser.cursor.byte_offset as _;
        Ok(ast::Varint(Spanned {
            item,
            span: Span { start, end },
        }))
    }
}

/// unsigned | varint-unsigned
struct LenType;
impl Rule for LenType {
    type Item = ast::LenType;
    const EXPECTED: Tree = Tree::Choice(&Unsigned::EXPECTED, &Varint::<Unsigned>::EXPECTED);

    fn parse(self, parser: &mut Parser) -> ParseResult<Self::Item> {
        if let Ok(x) = parser.match_rule(Unsigned) {
            return Ok(ast::LenType::Fixed(x));
        }
        if let Ok(x) = parser.match_rule(Varint(Unsigned)) {
            return Ok(ast::LenType::Varint(x));
        }
        Self::unmatched()
    }
}

#[test]
fn test_len_type() {
    LenType.parse(&mut Parser::new("u114514")).unwrap();
    LenType.parse(&mut Parser::new("@varint(u114514)")).unwrap();
    LenType
        .parse(&mut Parser::new("@varint (u1//hello\n )"))
        .unwrap();
}

/// ("@vector" | "@stream") ws* "(" ws* len-type ws* type ws* ")"
struct SequenceLike;
impl Rule for SequenceLike {
    type Item = ast::SequenceLike;
    const EXPECTED: Tree = Tree::Rule("sequence-like");

    fn parse(self, parser: &mut Parser) -> ParseResult<Self::Item> {
        let kind = if let Ok(_) = parser.match_str("@vector") {
            ast::SequenceKind::Vector
        } else if let Ok(_) = parser.match_str("@stream") {
            ast::SequenceKind::Stream
        } else {
            Err(Tree::Rule(r#""@vector" | "@stream""#))?
        };
        parser.ws();
        parser.match_char('(')?;
        parser.ws();
        let len_ty = parser.parse_span(LenType)?;
        parser.ws_plus()?;
        let elem_ty = parser.parse_span(Type)?;
        parser.ws();
        parser.match_char(')')?;
        Ok(ast::SequenceLike {
            kind,
            len_ty,
            elem_ty,
        })
    }
}

/// sequence-like | varint-signed | varint-unsigned | valid-name | builtin
struct Type;
impl Rule for Type {
    type Item = ast::Type;
    const EXPECTED: Tree = Tree::Rule("type");

    fn parse(self, parser: &mut Parser) -> ParseResult<Self::Item> {
        if let Ok(x) = parser.match_rule(SequenceLike) {
            return Ok(ast::Type::Sequence(Box::new(x)));
        }
        if let Ok(x) = parser.match_rule(Varint(Signed)) {
            return Ok(ast::Type::VarintSigned(x));
        }
        if let Ok(x) = parser.match_rule(Varint(Unsigned)) {
            return Ok(ast::Type::VarintUnsigned(x));
        }
        if let Ok(x) = parser.match_rule(ValidName) {
            return Ok(ast::Type::Ident(x));
        }
        if let Ok(x) = parser.match_rule(Builtin) {
            return Ok(ast::Type::Builtin(x));
        }
        Self::unmatched()
    }
}

#[test]
fn test_type() {
    let ty = Type.parse(&mut Parser::new("u114514")).unwrap();
    assert_eq!(
        ty,
        ast::Type::Builtin(ast::Builtin::Unsigned(ast::Unsigned(ast::Number::Value(
            114514
        ))))
    );
    let ty = Type.parse(&mut Parser::new("i23a")).unwrap();
    assert_eq!(ty, ast::Type::Ident("i23a".into()));
    let ty = Type
        .parse(&mut Parser::new("@vector(u8 @varint(i64))"))
        .unwrap();
    match ty {
        ast::Type::Sequence(x) => {
            assert_eq!(
                x.len_ty.item,
                ast::LenType::Fixed(ast::Unsigned(ast::Number::Value(8)))
            );
            match x.elem_ty.item {
                ast::Type::VarintSigned(ast::Varint(x)) => {
                    assert_eq!(x.item, ast::Signed(ast::Number::Value(64)))
                }
                _ => panic!(),
            }
        }
        _ => panic!(),
    }
}

/// "void" | signed | unsigned
struct Builtin;
impl Rule for Builtin {
    type Item = ast::Builtin;
    const EXPECTED: Tree = Tree::Rule("builtin");

    fn parse(self, parser: &mut Parser) -> ParseResult<Self::Item> {
        if let Ok(_) = parser.match_str("void") {
            return Ok(ast::Builtin::Void);
        }
        if let Ok(x) = parser.match_rule(Signed) {
            return Ok(ast::Builtin::Signed(x));
        }
        if let Ok(x) = parser.match_rule(Unsigned) {
            return Ok(ast::Builtin::Unsigned(x));
        }
        Self::unmatched()
    }
}

/// !(builtin ("{" | "@" | ")" | "}" | "=" | ws | END)) identifier
struct ValidName;
impl Rule for ValidName {
    type Item = Ident;
    const EXPECTED: Tree = Tree::Rule("builtin");

    fn parse(self, parser: &mut Parser) -> ParseResult<Self::Item> {
        parser.match_rule(Not((
            Builtin,
            choice!(
                Char::<'{'>,
                Char::<'@'>,
                Char::<')'>,
                Char::<'}'>,
                Char::<'='>,
                WHITE_SPACE,
                End
            ),
        )))?;
        parser.parse(Identifier)
    }
}

/// "@primitive" ws* "(" ws* builtin ws* ")" ws* typename
struct Primitive;
impl Rule for Primitive {
    type Item = ast::NamedDefinition;
    const EXPECTED: Tree = Tree::Rule("primitive");

    fn parse(self, parser: &mut Parser) -> ParseResult<Self::Item> {
        parser.match_str("@primitive")?;
        parser.ws();
        parser.match_char('(')?;
        parser.ws();
        let ty = parser.parse_span(Builtin)?;
        parser.ws();
        parser.match_char(')')?;
        parser.ws();
        let name = parser.parse_span(ValidName)?;
        Ok(ast::NamedDefinition {
            name,
            def: ast::Definition::Primitive(ty),
        })
    }
}

/// ident ws* ":" ws* type
struct Member;
impl Rule for Member {
    type Item = ast::Member;
    const EXPECTED: Tree = Tree::Rule("member");

    fn parse(self, parser: &mut Parser) -> ParseResult<Self::Item> {
        let name = parser.parse_span(Identifier)?;
        parser.ws();
        parser.match_char(':')?;
        parser.ws();
        let ty = parser.parse_span(Type)?;
        Ok(ast::Member { name, ty })
    }
}

#[test]
fn test_member() {
    (Member, End).parse(&mut Parser::new("x :x")).unwrap();
    (IndexedMember, End)
        .parse(&mut Parser::new("x: x= 1"))
        .unwrap();
    (IndexedMember, End)
        .parse(&mut Parser::new("xxx =1"))
        .unwrap();
    (IndexedMember, End)
        .parse(&mut Parser::new("xxx = 0xf2"))
        .unwrap();
}

/// ("@strcut" | "@packed") ws+ valid-name ws* "{" (ws* member)* ws* "}"
struct StructLike;
impl Rule for StructLike {
    type Item = ast::NamedDefinition;
    const EXPECTED: Tree = Tree::Rule("struct | packed");

    fn parse(self, parser: &mut Parser) -> ParseResult<Self::Item> {
        let kind = if let Ok(_) = parser.match_str("@struct") {
            ast::StructKind::Struct
        } else if let Ok(_) = parser.match_str("@packed") {
            ast::StructKind::Packed
        } else {
            Err(Tree::Rule(r#""@strcut" | "@packed""#))?
        };
        parser.ws_plus()?;
        let name = parser.parse_span(ValidName)?;
        parser.ws();
        parser.match_char('{')?;
        parser.ws();
        let mut members = vec![];
        loop {
            parser.ws();
            let member = match parser.match_span(Member) {
                Ok(x) => x,
                Err(_) => break,
            };
            members.push(member);
        }
        parser.ws();
        parser.match_char('}')?;
        Ok(ast::NamedDefinition {
            name,
            def: ast::Definition::StructLike { kind, members },
        })
    }
}

/// indent ws* (":" ws* type)? ws* ("=" ws* number)?
struct IndexedMember;
impl Rule for IndexedMember {
    type Item = ast::IndexedMember;
    const EXPECTED: Tree = Tree::Rule("member");

    fn parse(self, parser: &mut Parser) -> ParseResult<Self::Item> {
        let name = parser.parse_span(Identifier)?;
        parser.ws();
        let ty = parser
            .match_char(':')
            .and_then(|_| {
                parser.ws();
                parser.match_span(Type)
            })
            .ok();
        parser.ws();
        let index = parser
            .match_char('=')
            .and_then(|_| {
                parser.ws();
                parser.match_span(Number)
            })
            .ok();
        Ok(ast::IndexedMember { name, ty, index })
    }
}

/// ("@enum" | "@dict") ws* "(" ws* unsigned ws* ")" ws* valid-name ws* "{" (ws* indexed-member)* ws * "}"
struct IndexedStructLike;
impl Rule for IndexedStructLike {
    type Item = ast::NamedDefinition;
    const EXPECTED: Tree = Tree::Rule("enum | dict");

    fn parse(self, parser: &mut Parser) -> ParseResult<Self::Item> {
        let kind = if let Ok(_) = parser.match_str("@enum") {
            ast::IndexedStructKind::Enum
        } else if let Ok(_) = parser.match_str("@dict") {
            ast::IndexedStructKind::Dict
        } else {
            Err(Tree::Rule(r#""@enum" | "@dict""#))?
        };
        parser.ws();
        parser.match_char('(')?;
        parser.ws();
        let index_ty = parser.parse_span(Unsigned)?;
        parser.match_char(')')?;
        parser.ws();
        let name = parser.parse_span(ValidName)?;
        parser.ws();

        parser.match_char('{')?;
        parser.ws();
        let mut members = vec![];
        loop {
            parser.ws();
            let member = match parser.match_span(IndexedMember) {
                Ok(x) => x,
                Err(_) => break,
            };
            members.push(member);
        }
        parser.ws();
        parser.match_char('}')?;
        Ok(ast::NamedDefinition {
            name,
            def: ast::Definition::IndexedStructLike {
                kind,
                index_ty,
                members,
            },
        })
    }
}

/// primitive | struct-like | indexed-struct-like
type Definition = variant_type!(Primitive, StructLike, IndexedStructLike);
const DEFINITION: Definition = variant!(Primitive, StructLike, IndexedStructLike);

/// ws* (definition ws*)* END
pub struct Document;
impl Rule for Document {
    type Item = ast::Document;
    const EXPECTED: Tree = unreachable!();

    fn parse(self, parser: &mut Parser) -> ParseResult<Self::Item> {
        parser.ws();
        let mut definitions = vec![];
        let err = loop {
            match parser.match_span(DEFINITION) {
                Ok(x) => {
                    definitions.push(x);
                    parser.ws();
                }
                Err(e) => break e,
            }
        };
        parser.match_rule(End).map_err(|_| err)?;
        Ok(ast::Document { definitions })
    }
}

#[test]
fn test_definition() {
    DEFINITION
        .parse(&mut Parser::new("@struct aaa {x:x} "))
        .unwrap();
    Document
        .parse(&mut Parser::new("@struct aaa {x:x} //"))
        .unwrap();
}
