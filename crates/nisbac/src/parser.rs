use std::ops::{Range, RangeInclusive};

use either::Either;
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Span {
    start: u32,
    end: u32,
}

impl Span {
    pub fn spanned<T>(self) -> impl FnOnce(T) -> Spanned<T> {
        #[inline]
        move |item| Spanned { item, span: self }
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

pub struct Parser<'a> {
    cursor: Cursor<'a>,
}

impl<'a> Parser<'a> {
    pub fn new(source: &'a str) -> Self {
        Self {
            cursor: Cursor {
                byte_offset: 0,
                source,
            },
        }
    }
    fn parse<T: Rule>(&mut self, rule: T) -> ParseResult<T::Item> {
        rule.parse(self)
    }
    fn parse_span<T: Rule>(&mut self, rule: T) -> ParseResult<Spanned<T::Item>> {
        let start = self.cursor.byte_offset;
        rule.parse(self).map(|item| Spanned {
            item,
            span: Span {
                start: start as u32,
                end: self.cursor.byte_offset as u32,
            },
        })
    }
    fn match_rule<T: Rule>(&mut self, rule: T) -> ParseResult<T::Item> {
        let start = self.cursor.byte_offset;
        self.parse(rule)
            .inspect_err(|_| self.cursor.byte_offset = start)
    }
    fn match_span<T: Rule>(&mut self, rule: T) -> ParseResult<Spanned<T::Item>> {
        let start = self.cursor.byte_offset as _;
        rule.parse(self)
            .inspect_err(|_| self.cursor.byte_offset = start)
            .map(|item| Spanned {
                item,
                span: Span {
                    start: start as u32,
                    end: self.cursor.byte_offset as u32,
                },
            })
    }
    fn match_char(&mut self, expected: char) -> ParseResult<()> {
        if self.cursor.peek() == Some(expected) {
            self.cursor.next();
            Ok(())
        } else {
            Err(Tree::Char(expected))?
        }
    }
    fn match_str(&mut self, expected: &'static str) -> ParseResult<()> {
        if self.cursor.remaining().starts_with(expected) {
            self.cursor.byte_offset += expected.len();
            Ok(())
        } else {
            Err(Tree::Str(expected))?
        }
    }
    fn match_set(&mut self, expected: &'static str) -> ParseResult<char> {
        match self.cursor.peek() {
            Some(x) if expected.contains(x) => {
                self.cursor.next();
                Ok(x)
            }
            _ => Err(Tree::Set(expected))?,
        }
    }
}

#[derive(Clone, Copy)]
struct Cursor<'a> {
    byte_offset: usize,
    source: &'a str,
}

impl<'a> Cursor<'a> {
    fn remaining(&self) -> &'a str {
        unsafe {
            str::from_utf8_unchecked(self.source.as_bytes().get_unchecked(self.byte_offset..))
        }
    }
    fn peek(&self) -> Option<char> {
        self.remaining().chars().next()
    }
    fn next(&mut self) -> Option<char> {
        let mut iter = self.remaining().chars();
        let r = iter.next();
        self.byte_offset = self.source.len() - iter.as_str().len();
        r
    }
}

type ParseResult<T> = Result<T, Unmatched>;

#[derive(Error, Debug)]
#[error("syntax error: expected {expected}")]
pub struct Unmatched {
    #[from]
    expected: Tree,
}

#[derive(Error, Debug)]
pub enum Tree {
    #[error("'{0}'")]
    Char(char),
    #[error("{0:?}")]
    Range(Range<char>),
    #[error("{0:?}")]
    RangeInclusive(RangeInclusive<char>),
    #[error("\"{0}\"")]
    Str(&'static str),
    #[error("[{}]", _0.escape_debug())]
    Set(&'static str),
    #[error("{0}")]
    Rule(&'static str),
    #[error("{0} | {1}")]
    Choice(&'static Tree, &'static Tree),
    #[error("!{0}")]
    Not(&'static Tree),
    #[error("end of input")]
    End,
}

trait Rule {
    type Item: Sized;
    const EXPECTED: Tree;
    fn parse(self, parser: &mut Parser) -> ParseResult<Self::Item>;
    fn unmatched<T>() -> Result<T, Unmatched> {
        Err(Unmatched {
            expected: Self::EXPECTED,
        })
    }
}

const fn map<T: Rule + Sized, R, F: FuncOnce<T::Item, Return = R>>(rule: T, func: F) -> Map<T, F> {
    Map { rule, func }
}

struct Map<T, F> {
    rule: T,
    func: F,
}

impl<T: Rule, F: FuncOnce<T::Item>> Rule for Map<T, F> {
    type Item = F::Return;
    const EXPECTED: Tree = T::EXPECTED;

    fn parse(self, parser: &mut Parser) -> ParseResult<Self::Item> {
        self.rule.parse(parser).map(|x| self.func.apply(x))
    }
}

trait FuncOnce<A> {
    type Return;
    fn apply(self, arg: A) -> Self::Return;
}

impl<A, R, F: FnOnce(A) -> R> FuncOnce<A> for F {
    type Return = R;
    fn apply(self, arg: A) -> Self::Return {
        (self)(arg)
    }
}

struct EitherIntoInner;
impl<T> FuncOnce<Either<T, T>> for EitherIntoInner {
    type Return = T;
    fn apply(self, arg: Either<T, T>) -> Self::Return {
        arg.into_inner()
    }
}

#[derive(Error, Debug)]
#[error("expected {expected} at {offset}")]
pub struct ParseError {
    pub offset: usize,
    pub expected: Tree,
}

pub fn parse(source: &str) -> Result<ast::Document, ParseError> {
    let mut parser = Parser::new(source);
    parser
        .parse(rules::Document)
        .map_err(|Unmatched { expected }| ParseError {
            offset: parser.cursor.byte_offset,
            expected,
        })
}

pub mod ast;
mod rules;
