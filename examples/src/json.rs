use std::{
    fmt::{self, Display, Formatter, Write as _},
    str::FromStr as _,
};

mod encode {
    include!(concat!(env!("OUT_DIR"), "/json_encode.rs"));
}
mod decode {
    include!(concat!(env!("OUT_DIR"), "/json_decode.rs"));
}

struct Parser<'a> {
    src: &'a [u8],
}

impl Parser<'_> {
    fn peek(&mut self) -> Option<u8> {
        self.src.first().copied()
    }
    fn next(&mut self) -> Option<u8> {
        let res = self.src.first().copied();
        self.src = &self.src[1..];
        res
    }
    fn skip_ws(&mut self) {
        if let Some(b' ' | b'\t' | b'\r' | b'\n') = self.peek() {
            self.src = &self.src[1..];
            self.skip_ws();
        }
    }
    fn match_str(&mut self, str: &str) {
        if self.src.starts_with(str.as_bytes()) {
            self.src = &self.src[str.len()..];
        } else {
            panic!("expected literal {}", str)
        }
    }
    // only for simple numbers
    fn parse_number(&mut self) -> f64 {
        let src = self.src;
        let mut len = 1;
        self.next();
        loop {
            match self.peek() {
                Some(b'0'..=b'9' | b'.') => {
                    len += 1;
                    self.src = &self.src[1..];
                }
                _ => break f64::from_str(str::from_utf8(&src[..len]).unwrap()).unwrap(),
            }
        }
    }
    /// escape is not supported
    fn parse_string(&mut self) -> encode::String {
        let len = self.src[1..].iter().position(|&x| x == b'"').unwrap();
        let data = self.src[1..len + 1].into();
        self.src = &self.src[len + 2..];
        encode::String { data }
    }
    fn parse_json(&mut self) -> encode::Json {
        self.skip_ws();
        match self.peek() {
            Some(x) => match x {
                b'f' => {
                    self.match_str("false");
                    encode::Json::False
                }
                b't' => {
                    self.match_str("true");
                    encode::Json::True
                }
                b'0'..=b'9' | b'-' => encode::Json::Number(self.parse_number()),
                b'"' => encode::Json::String(self.parse_string()),
                b'[' => encode::Json::Array(self.parse_array()),
                b'{' => encode::Json::Object(self.parse_object()),
                b'n' => {
                    self.match_str("null");
                    encode::Json::Null
                }
                x => panic!("expected json value, found '{}'", x as char),
            },
            None => panic!("expected json value"),
        }
    }
    fn parse_array(&mut self) -> encode::Array {
        self.next();
        self.skip_ws();
        let mut items = vec![];
        match self.peek() {
            Some(b']') => {
                self.next();
                encode::Array { items }
            }
            _ => loop {
                items.push(self.parse_json());
                self.skip_ws();
                match self.next() {
                    Some(b']') => break encode::Array { items },
                    Some(b',') => {}
                    _ => panic!("expected ']' or ','"),
                }
            },
        }
    }
    fn parse_entry(&mut self) -> encode::Entry {
        self.skip_ws();
        match self.peek() {
            Some(b'"') => {
                let key = self.parse_string();
                self.skip_ws();
                if self.next() != Some(b':') {
                    panic!("expected ':'")
                }
                self.skip_ws();
                let value = self.parse_json();
                encode::Entry { key, value }
            }
            _ => panic!("expected string"),
        }
    }
    fn parse_object(&mut self) -> encode::Object {
        self.next();
        self.skip_ws();
        let mut entries = vec![];
        match self.peek() {
            Some(b'}') => {
                self.next();
                encode::Object { entries }
            }
            _ => loop {
                entries.push(self.parse_entry());
                self.skip_ws();
                match self.next() {
                    Some(b'}') => break encode::Object { entries },
                    Some(b',') => {}
                    _ => panic!("expected '}}' or ','"),
                }
            },
        }
    }
}

impl Display for decode::Json<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            decode::Json::False => f.write_str("false"),
            decode::Json::True => f.write_str("true"),
            &decode::Json::Number(x) => write!(f, "{x}"),
            decode::Json::String(x) => {
                write!(f, "{:?}", unsafe {
                    str::from_utf8_unchecked(x.data.as_bytes())
                })
            }
            decode::Json::Array(x) => write!(f, "{x}"),
            decode::Json::Object(x) => write!(f, "{x}"),
            decode::Json::Null => f.write_str("null"),
        }
    }
}

impl Display for decode::Array<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_char('[')?;
        let mut iter = self.items;
        while let Some(x) = iter.next() {
            write!(f, "{}", x.unwrap())?;
            if !iter.as_bytes().is_empty() {
                f.write_char(',')?;
            }
        }
        f.write_char(']')
    }
}

impl Display for decode::Object<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_char('{')?;
        let mut iter = self.entries;
        while let Some(x) = iter.next() {
            let decode::Entry { key, value } = x.unwrap();
            write!(f, "{:?}:{value}", unsafe {
                str::from_utf8_unchecked(key.data.as_bytes())
            })?;
            if !iter.as_bytes().is_empty() {
                f.write_char(',')?;
            }
        }
        f.write_char('}')
    }
}

fn main() {
    let input = r#"{
        "meta": {
            "version": 1.2,
            "valid": true,
            "tags": ["rust", "json", "parser", null]
        },
        "user": {
            "id": 123456,
            "name": "Alice",
            "email": "alice@example.com",
            "active": false,
            "scores": [99, 87.5, 92, 100],
            "profile": {
                "age": 30,
                "languages": ["en", "zh", "fr"],
                "address": {
                    "country": "Wonderland",
                    "zip": "00000"
                }
            }
        },
        "items": [
            {
                "id": 1,
                "name": "item1",
                "price": 10.5,
                "available": true
            },
            {
                "id": 2,
                "name": "item2",
                "price": -3.14,
                "available": false,
                "extra": null
            },
            {
                "id": 3,
                "name": "item3",
                "nested": [
                    [1, 2],
                    [3, 4, [5, 6]]
                ]
            }
        ],
        "empty_array": [],
        "empty_object": {},
        "mixed": [
            null,
            true,
            false,
            0,
            -1,
            3.14159,
            "text",
            {
                "inner": ["a", "b", {"deep": "value"}]
            }
        ]
    }"#;
    let json = Parser {
        src: input.as_bytes(),
    }
    .parse_json();

    let data = nisba::encode::encode(&json).unwrap();

    let json: decode::Json = nisba::decode::decode(&data).unwrap();
    println!("{json}");
}
