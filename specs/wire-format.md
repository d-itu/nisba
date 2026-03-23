# Nisba Wire Format Specification

## Version 0.1

---

## Overview

Nisba is a schema-driven binary object serialization format with the following properties:

- No runtime type information in wire format
- Zero-copy friendly
- No implicit padding

Nisba does not require canonical encoding.
Multiple valid encodings **MAY** exist for the same logical value:

- Unused bits in bitmaps and varints are not required to be zero.

Schemas are used for defining data types.
Schemas can be defined with:

- A nisba DSL file
- A binary nisba schema file

---

## Type System

Nisba types are divided into:

- Fundamental Types
  - `integer`
  - `primitive`
  - `varint`
- Containers
  - `vector`
  - `stream`
- Composite Types
  - `packed`
  - `struct`
  - `enum`
  - `dict`

Some types can be `fixed-sized`, whose size are known at compilation time.

Composite types are defined by user in external schemas.

Recursive type definitions are allowed only if recursion is guarded
by a variable-size container that introduces an explicit boundary,
such as vector or stream.

Unbounded recursion without such a boundary is invalid.

---

### Integer

An integer is defined as:

```
uN / iN
```

Where:

- `N` is **any** non-negative integer bit width.
- Maximum bit width is 65535
- Signed integers use two's complement.
- Integers are encoded in little endian.
- Bit order inside byte is LSB-first.
- Integers as member of composite types must be byte-aligned, except in `packed`.

---

### Varint

Varint encodes integers in a variable-length format:

- Encoded integer must have a bit width of 16, 32 or 64.
- Unsigned integers are encoded into LEB128 format.
- Signed integers are ZigZag transformed, then encoded as unsigned varint.

When decoding:

- Additional bits of the last encoded byte are ignored.
- Longer bytes than required is treated as an error.

---

### Primitive

A primitive is a semantic newtype over a byte-aligned integer type.

Primitives are encoded as their underlying integer type.

---

### Vector

Vector is a sequence of fixed-sized elements with **length** prefix:

```
[length] [elements...]
```

Where:

- Length encodes number of elements.
- Length type must be:
  - non-zero byte-aligned unsigned `integer`s with a maximum bit width of 64
  - valid unsigned `varint`s
- Elements must be fixed-sized.

Vectors support random access.

---

### Stream

Stream is a sequence of elements with **byte-size** prefix.

```
[byte_length] [raw bytes...]
```

Where:

- Length indicates number of bytes following.
- Length type is same with `vector`'s length type.
- Element type in stream may have variable size.

### Packed

Packed defines a packed structure which supports bit-field:

```
[field1] [field2] ...
```

Where:

- Members must be fixed-sized types.
- Total bit size must be multiple of 8.
- Bit numbering follows LSB-first within byte.

---

### Struct

Struct defines a structure where fields are serialized in declaration order:

```
[field1] [field2] ...
```

---

### Enum

Enum is a tagged union holding exactly one member at a time
with a tag prefix indicating the active member:

```
[tag] [variant data]
```

Where:

- Tag type must be non-zero byte-aligned integers with a maximum bit-width of 64.
- Each variant must have a unique tag value.
- Unknown Tag value in decoding is treated as an error.

---

### Dict

Dict represents optional fields using bitmap:

```
[bitmap] [present fields in order]
```

- Bitmap uses the same types with Enum tag.
- Each bit corresponds to field index.
- Unused bits are ignored.

---

## Concepts

### Fixed-Sized

A type is fixed-sized if and only if it is:

- `integer`
- `primitive`
- `packed`
- `enum`, if and only if:
  - all variants are fixed-sized, and
  - all variants have the same encoded size
- `dict`, if and only if all fields have a fixed size of 0

### Byte-Aligned

A type is byte-aligned if it has a fixed size and which is multiple of 8.
