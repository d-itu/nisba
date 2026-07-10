# Nisba Schema Language Specification

## Version 0.1

---

## Overview

Nisba schema language is a DSL used for defining types for the Nisba Binary Message Format.

A nisba file consists of definitions of:

- `extern`
- `packed`
- `struct`
- `enum`
- `dict`

---

## File Extension

Nisba files use extension `.nisba`.

---

## Comments

Single-line comments start with dobule slash.

```
// comments
```

---

## Identifiers

Identifiers are used in definitions for type names and member names.

Identifiers must match

```regex
[a-zA-Z][a-zA-Z0-9_]*
```

Identifiers are case-sensitive.
`snake-case` identifiers are suggested.

---

## Type Expressions

Type expressions are used in composite type members and sub-expressions. They are one of:

- builtin types
- inline expressions
- identifiers (user-defined types)

---

## Builtin Types

### Integer Types

```
uN // unsigned
iN // signed
```

Where:

N is a non-negative integer.

### Special Types

```
void
```

`void` is an alias of u0.

---

## Inline expressions

The following inline type constructors are supported:

---

### Varint

```
@varint(<integer_type>)
```

Example:

```
@varint(u64)
```

---

### Vector & Stream

```
@vector(<length_type> <element_type>)
@stream(<length_type> <element_type>)
```

Example:

```
@vector(u8 u8)
@stream(@varint(u64) string)
```

`<length-type>` must be a valid length type:

- `u8`, `u16`, `u32`, `u64`
- `@varint(u16)`, `@varint(u32)`, `@varint(u64)`

---

## Definitions

### External

```
@extern(<integer>) <name>
@extern <name>
```

Example:

```
@extern(u32) f32
@extern message
```

### Struct & Packed

```
@packed|@struct <name> {
  <field>: <type>
  ...
}

```

Examples:

```
@packed rgb {
  r: f32
  g: f32
  b: f32
}

@struct person {
  id: u64
  name: string
}
```

### Enum & Dict

```
@enum|@dict (<index-type>) <name> {
  <field>: <type> = <integer>
  ...
}

```

Where:

- `<index-type>` is used as tag type in `enum` or bitmap type in `dict`.
- `<integer>` is an integer literal used as tag in `enum` or field index in `dict`.
- `: <type>` and ` = <integer>` are optional.
- omitted types default to `void`.
- tag / index numbers are assigned like in `C` programming language.

Examples:

```
@enum(u8) color {
  black
  white: void = 1
  rgb: rgb
  transparent = 0xff
}
```

### Resolution

- Type names are globally unique
- Member names are unique in each definition
- Built-in types cannot be redefined
- Definitions have no order dependency
- Type recursions should be validated

### Grammar

Parsing expression grammar(PEG) is used to formally describe nisba syntax.

```
doc := BEGIN ws* (definition ws*)* END
definition := extern | struct-like | indexed-struct-like

extern := "@extern" ws* ("(" ws* builtin ws* ")" ws*)? valid-name

struct-like := ("@strcut" | "@packed") ws+ valid-name ws* "{" (ws* member)* ws* "}"
member := identifier ws* ":" ws* type

indexed-struct-like := ("@enum" | "@dict") ws* "(" ws* unsigned ws* ")" ws* valid-name ws* "{" (ws* indexed-member)* ws * "}"
indexed-member := indentifier ws* (":" ws* type)? ws* ("=" ws* number)?

type := sequence-like | varint-signed | varint-unsigned | valid-name | builtin

sequence-like := ("@vector" | "@stream") ws* "(" ws* len-type ws* type ws* ")"
len-type := unsigned | varint-unsigned

varint-unsigned := "@varint" ws* "(" ws* unsigned ws* ")"
varint-signed := "@varint" ws* "(" ws* signed ws* ")"

unsigned := "u" packed-dec
signed := "i" packed-dec
packed-dec := "0" | [1-9] [0-9]*

number := bin | hex | dec
bin := "0" [bB] [_]* [01] [01_]*
hex := "0" [xX] [_]* [0-9a-fA-F] [0-9a-fA-F_]*
dec := [0-9] [0-9_]*

valid-name := !(builtin ("{" | "@" | ")" | "}" | "=" | ws | END)) identifier
builtin := "void" | signed | unsigned

identifier := [a-zA-Z] [a-zA-Z0-9_]*
ws := [ \r\n\t] | comment
comment := "//" (!("\n" | END) .)*
```
