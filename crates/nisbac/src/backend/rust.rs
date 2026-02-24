use crate::schema::{Definition, Schema};

pub struct Config {}

pub fn generate(schema: &Schema, config: &Config) {
    for definition in schema.definitions() {
        match definition {
            Definition::Primitive(_) => {}
            Definition::Vector(_) => {}
            Definition::Stream(_) => {}
            Definition::Packed(packed) => todo!(),
            Definition::Struct(s) => todo!(),
            Definition::Enum(e) => todo!(),
            Definition::Dict(dict) => todo!(),
        }
    }
}
