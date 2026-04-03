use std::hint;

use criterion::{Criterion, criterion_group, criterion_main};
use protobuf::{AsMut, Parse as _, Serialize as _};
use protobuf_well_known_types::Empty;
use sonic_rs::{
    Deserializer, JsonContainerTrait as _, JsonNumberTrait, JsonType, JsonValueTrait, Read, Value,
};

pub enum JsonValue {
    Null,
    False,
    True,
    Number(f64),
    String(Vec<u8>),
    Array(Vec<JsonValue>),
    Object(Vec<(Vec<u8>, JsonValue)>),
}

pub mod nisba_encode {
    include!(concat!(env!("OUT_DIR"), "/json_encode.rs"));
}
pub mod nisba_decode {
    use std::fmt::{self, Display, Formatter, Write as _};

    use super::JsonValue;

    include!(concat!(env!("OUT_DIR"), "/json_decode.rs"));

    impl Json<'_> {
        pub fn de(&self) -> JsonValue {
            match self {
                Json::False => JsonValue::False,
                Json::True => JsonValue::True,
                &Json::Number(x) => JsonValue::Number(x),
                Json::String(x) => JsonValue::String(x.data.as_bytes().into()),
                Json::Array(x) => JsonValue::Array(x.items.iter().map(Self::de).collect()),
                Json::Object(object) => JsonValue::Object(
                    object
                        .entries
                        .iter()
                        .map(|Entry { key, value }| (key.data.as_bytes().into(), value.de()))
                        .collect(),
                ),
                Json::Null => JsonValue::Null,
            }
        }
        pub fn visit(&self) {
            match self {
                Json::False => {}
                Json::True => {}
                Json::Number(_) => {}
                Json::String(_) => {}
                Json::Array(x) => {
                    for item in &x.items {
                        item.visit();
                    }
                }
                Json::Object(x) => {
                    for entry in &x.entries {
                        entry.value.visit();
                    }
                }
                Json::Null => {}
            }
        }
    }

    impl Display for Json<'_> {
        fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
            match self {
                Json::False => f.write_str("false"),
                Json::True => f.write_str("true"),
                &Json::Number(x) => write!(f, "{x}"),
                Json::String(x) => {
                    write!(f, "{:?}", str::from_utf8(x.data.as_bytes()).unwrap())
                }
                Json::Array(x) => write!(f, "{x}"),
                Json::Object(x) => write!(f, "{x}"),
                Json::Null => f.write_str("null"),
            }
        }
    }

    impl Display for Array<'_> {
        fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
            f.write_char('[')?;
            let mut iter = self.items.iter();
            while let Some(x) = iter.next() {
                write!(f, "{x}")?;
                if iter.len() != 0 {
                    f.write_char(',')?;
                }
            }
            f.write_char(']')
        }
    }

    impl Display for Object<'_> {
        fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
            f.write_char('{')?;
            let mut iter = self.entries.iter();
            while let Some(Entry { key, value }) = iter.next() {
                write!(f, "{:?}:{value}", unsafe {
                    str::from_utf8_unchecked(key.data.as_bytes())
                })?;
                if iter.len() != 0 {
                    f.write_char(',')?;
                }
            }
            f.write_char('}')
        }
    }
}

pub mod proto {
    use std::fmt::{self, Display, Formatter, Write as _};

    use crate::proto::json::{ValueCase, ValueOneof};

    use super::JsonValue;

    include!(concat!(env!("OUT_DIR"), "/protobuf_generated/generated.rs"));

    impl Json {
        pub fn de(self) -> JsonValue {
            match self.value_case() {
                ValueCase::False => JsonValue::False,
                ValueCase::True => JsonValue::True,
                ValueCase::Number => JsonValue::Number(self.number()),
                ValueCase::String => todo!(),
                ValueCase::Array => todo!(),
                ValueCase::Object => todo!(),
                ValueCase::Null => todo!(),
                ValueCase::not_set => todo!(),
            }
        }
    }

    impl JsonView<'_> {
        pub fn visit(&self) {
            match self.value() {
                ValueOneof::False(_) => {}
                ValueOneof::True(_) => {}
                ValueOneof::Number(_) => {}
                ValueOneof::String(_) => {}
                ValueOneof::Array(arr) => {
                    for item in arr.items() {
                        item.visit();
                    }
                }
                ValueOneof::Object(obj) => {
                    for entry in obj.entries() {
                        entry.value().visit();
                    }
                }
                ValueOneof::Null(_) => {}
                ValueOneof::not_set(_) => unreachable!(),
            }
        }
    }

    impl Display for JsonView<'_> {
        fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
            match self.value() {
                ValueOneof::False(_) => f.write_str("false"),
                ValueOneof::True(_) => f.write_str("true"),
                ValueOneof::Number(x) => write!(f, "{x}"),
                ValueOneof::String(x) => write!(f, "{:?}", str::from_utf8(x).unwrap()),
                ValueOneof::Array(x) => write!(f, "{x}"),
                ValueOneof::Object(x) => write!(f, "{x}"),
                ValueOneof::Null(_) => f.write_str("null"),
                ValueOneof::not_set(_) => unreachable!(),
            }
        }
    }

    impl Display for ArrayView<'_> {
        fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
            f.write_char('[')?;
            let mut iter = self.items().iter();
            while let Some(x) = iter.next() {
                write!(f, "{x}")?;
                if iter.len() != 0 {
                    f.write_char(',')?;
                }
            }
            f.write_char(']')
        }
    }

    impl Display for ObjectView<'_> {
        fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
            f.write_char('{')?;
            let mut iter = self.entries().iter();
            while let Some(entry) = iter.next() {
                write!(
                    f,
                    "{:?}:{}",
                    str::from_utf8(entry.key()).unwrap(),
                    entry.value()
                )?;
                if iter.len() != 0 {
                    f.write_char(',')?;
                }
            }
            f.write_char('}')
        }
    }
}

pub mod bincode {
    #[derive(bincode_next::Encode)]
    pub enum Json {
        Null,
        False,
        True,
        Number(f64),
        String(Vec<u8>),
        Array(Vec<Json>),
        Object(Vec<(Vec<u8>, Json)>),
    }
    #[derive(bincode_next::BorrowDecode)]
    pub enum JsonRef<'a> {
        Null,
        False,
        True,
        Number(f64),
        String(&'a [u8]),
        Array(Vec<JsonRef<'a>>),
        Object(Vec<(&'a [u8], JsonRef<'a>)>),
    }
    impl JsonRef<'_> {
        pub fn visit(&self) {
            match self {
                JsonRef::Null => {}
                JsonRef::False => {}
                JsonRef::True => {}
                JsonRef::Number(_) => {}
                &JsonRef::String(_) => {}
                JsonRef::Array(x) => x.iter().for_each(Self::visit),
                JsonRef::Object(x) => x.iter().for_each(|(_, v)| v.visit()),
            }
        }
    }
}

const TEST_DATA: &str = include_str!("data/citm_catalog.json");

fn nisba(value: &Value) -> nisba_encode::Json {
    use nisba_encode::{Array, Entry, Json, Object, String};
    match value.get_type() {
        JsonType::Null => Json::Null,
        JsonType::Boolean => match value.as_bool().unwrap() {
            true => Json::True,
            false => Json::False,
        },
        JsonType::Number => Json::Number(value.as_number().unwrap().as_f64().unwrap()),
        JsonType::String => Json::String(String {
            data: value.as_str().unwrap().as_bytes().into(),
        }),
        JsonType::Object => Json::Object(Object {
            entries: value
                .as_object()
                .unwrap()
                .iter()
                .map(|(k, v)| Entry {
                    key: String {
                        data: k.as_bytes().into(),
                    },
                    value: nisba(v),
                })
                .collect(),
        }),
        JsonType::Array => Json::Array(Array {
            items: value.as_array().unwrap().iter().map(nisba).collect(),
        }),
    }
}

fn proto(value: &Value) -> proto::Json {
    use proto::{Array, Entry, Json, Object};
    use protobuf::Repeated;
    let mut json = Json::new();
    let empty = Empty::new();
    match value.get_type() {
        JsonType::Null => json.set_null(empty),
        JsonType::Boolean => match value.as_bool().unwrap() {
            true => json.set_true(empty),
            false => json.set_false(empty),
        },
        JsonType::Number => json.set_number(value.as_number().unwrap().as_f64().unwrap()),
        JsonType::String => json.set_string(value.as_str().unwrap().as_bytes()),
        JsonType::Object => json.set_object({
            let mut obj = Object::new();
            obj.set_entries({
                let mut repeated = Repeated::new();
                repeated
                    .as_mut()
                    .extend(value.as_object().unwrap().iter().map(|(k, v)| {
                        let mut entry = Entry::new();
                        entry.set_key(k.as_bytes());
                        entry.set_value(proto(v));
                        entry
                    }));
                repeated
            });
            obj
        }),
        JsonType::Array => json.set_array({
            let mut arr = Array::new();
            arr.set_items({
                let mut repeated = Repeated::new();
                repeated
                    .as_mut()
                    .extend(value.as_array().unwrap().iter().map(proto));
                repeated
            });
            arr
        }),
    };
    json
}

fn bincode(value: &Value) -> bincode::Json {
    use bincode::*;
    match value.get_type() {
        JsonType::Null => Json::Null,
        JsonType::Boolean => match value.as_bool().unwrap() {
            true => Json::True,
            false => Json::False,
        },
        JsonType::Number => Json::Number(value.as_number().unwrap().as_f64().unwrap()),
        JsonType::String => Json::String(value.as_str().unwrap().as_bytes().into()),
        JsonType::Object => Json::Object(
            value
                .as_object()
                .unwrap()
                .iter()
                .map(|(k, v)| (k.as_bytes().into(), bincode(v)))
                .collect(),
        ),
        JsonType::Array => Json::Array(value.as_array().unwrap().iter().map(bincode).collect()),
    }
}

fn bench(c: &mut Criterion) {
    let value = Deserializer::new(Read::from(TEST_DATA))
        .deserialize()
        .unwrap();
    let nisba = nisba(&value);
    let proto = proto(&value);
    let bincode = bincode(&value);

    c.bench_function("nisba encode", |b| {
        b.iter(|| {
            hint::black_box(nisba::encode::encode(&nisba).unwrap());
        })
    });
    c.bench_function("protobuf encode", |b| {
        b.iter(|| {
            hint::black_box(proto.serialize().unwrap());
        })
    });
    c.bench_function("bincode encode", |b| {
        b.iter(|| {
            hint::black_box(
                bincode_next::encode_to_vec(
                    &bincode,
                    bincode_next::config::standard().with_variable_int_encoding(),
                )
                .unwrap(),
            )
        })
    });

    let nisba = nisba::encode::encode(&nisba).unwrap();
    let proto = proto.serialize().unwrap();
    let bincode = bincode_next::encode_to_vec(
        &bincode,
        bincode_next::config::standard().with_variable_int_encoding(),
    )
    .unwrap();

    dbg!(nisba.len());
    dbg!(proto.len());
    dbg!(bincode.len());

    c.bench_function("nisba decode+visit", |b| {
        b.iter(|| {
            hint::black_box(
                nisba::decode::decode::<nisba_decode::Json>(&nisba)
                    .unwrap()
                    .visit(),
            );
        })
    });
    c.bench_function("protobuf decode+visit", |b| {
        b.iter(|| {
            hint::black_box(proto::Json::parse(&proto).unwrap().as_view().visit());
        })
    });
    c.bench_function("bincode decode+visit", |b| {
        b.iter(|| {
            bincode_next::borrow_decode_from_slice::<bincode::JsonRef, _>(
                &bincode,
                bincode_next::config::standard(),
            )
            .unwrap()
            .0
            .visit()
        })
    });

    let nisba = nisba::decode::decode::<nisba_decode::Json>(&nisba).unwrap();
    let proto = proto::Json::parse(&proto).unwrap();
    assert_eq!(format!("{nisba}"), format!("{}", proto.as_view()));
}

criterion_group!(benches, bench);
criterion_main!(benches);
