use ahash::AHashSet;
use heck::ToUpperCamelCase;
use proc_macro2::{Literal, Span, TokenStream};
use quote::{ToTokens, quote};

use crate::{Ident, back::CodeGenKind, schema::*};

trait IdentExt {
    fn no_rename(&self) -> proc_macro2::Ident;
    fn camel_case(&self) -> proc_macro2::Ident;
}

fn gen_ident(s: &str) -> proc_macro2::Ident {
    let id = proc_macro2::Ident::new(s, Span::call_site());
    if let Ok(id) = syn::parse2::<proc_macro2::Ident>(quote!(#id)) {
        return id;
    }
    proc_macro2::Ident::new_raw(s, Span::call_site())
}

impl IdentExt for Ident {
    fn no_rename(&self) -> proc_macro2::Ident {
        gen_ident(self)
    }
    fn camel_case(&self) -> proc_macro2::Ident {
        let s = self.to_upper_camel_case();
        gen_ident(&s)
    }
}

#[allow(dead_code)]
pub struct Config {}

struct Context<'a> {
    schema: &'a Schema,
    has_lifetime: AHashSet<Ident>,
    #[allow(dead_code)]
    config: Config,
    kind: CodeGenKind,
}

pub fn generate(schema: &Schema, kind: CodeGenKind, config: Config) -> TokenStream {
    let mut ctx = Context {
        schema,
        has_lifetime: AHashSet::new(),
        config,
        kind,
    };
    let definitions = schema
        .definitions()
        .iter()
        .map(|definition| match definition {
            &Definition::Primitive(Primitive {
                ref name,
                bit_width,
            }) => {
                let size = Literal::usize_unsuffixed(bit_width as usize / 8);
                let name = name.no_rename();
                quote! {
                    ::nisba::const_assert_eq!(::core::mem::size_of::<#name>(), #size);
                }
            }
            Definition::Vector(_) => quote!(),
            Definition::Stream(_) => quote!(),
            Definition::Packed(packed) => packed.generate(&mut ctx),
            Definition::Struct(s) => s.generate(&mut ctx),
            Definition::Enum(e) => e.generate(&mut ctx),
            Definition::Dict(dict) => dict.generate(&mut ctx),
        });
    quote! {
        #(#definitions)*
    }
}

impl Packed {
    fn is_bitfield(&self, ctx: &Context) -> bool {
        self.members.iter().any(|member| {
            member
                .ty
                .bit_width(&ctx.schema)
                .fixed_byte_aligned()
                .is_err()
        })
    }
    fn generate(&self, ctx: &mut Context) -> TokenStream {
        match self.is_bitfield(ctx) {
            true => self.generate_bitfield(ctx),
            false => self.generate_struct(ctx),
        }
    }
    fn generate_bitfield(&self, ctx: &Context) -> TokenStream {
        let bit_size: usize = self
            .members
            .iter()
            .map(|member| member.ty.bit_width(&ctx.schema).fixed().unwrap())
            .sum();
        let byte_size = Literal::usize_unsuffixed(bit_size / 8);
        let name = self.name.camel_case();
        let impls = match ctx.kind {
            CodeGenKind::Encode => impl_encode(
                &name,
                &quote!(),
                quote!(self.0.len()),
                quote! {
                    unsafe { w.push_bytes(&self.0) }
                },
                quote!(const),
            ),
            CodeGenKind::Decode => impl_decode(
                &name,
                None,
                quote! {
                    Self(unsafe { *::nisba::const_try!(r.next_bytes(::core::mem::size_of::<Self>())).as_ptr().cast() })
                },
            ),
        };
        quote! {
            #[repr(transparent)]
            pub struct #name(pub [u8; #byte_size]);
            #impls
        }
    }
    fn generate_struct(&self, ctx: &mut Context) -> TokenStream {
        let name = self.name.camel_case();
        let body = StructGen::generate(&self.name, self.members.iter(), ctx, false).quote();
        let impls = match ctx.kind {
            CodeGenKind::Encode => impl_encode(
                &name,
                &quote!(),
                quote!(::core::mem::size_of::<Self>()),
                quote! {
                    unsafe {
                        w.push_bytes(::core::slice::from_raw_parts(
                            &raw const *self as _,
                            ::core::mem::size_of::<Self>()
                        ))
                    }
                },
                quote!(const),
            ),
            CodeGenKind::Decode => impl_decode(&name, None, decode_primitive(quote!(Self))),
        };
        quote! {
            #[repr(packed)]
            #body
            #impls
        }
    }
}

impl Struct {
    fn generate(&self, ctx: &mut Context) -> TokenStream {
        let res = StructGen::generate(&self.name, self.members.iter(), ctx, false);
        let ty = res.quote();
        let impls = match ctx.kind {
            CodeGenKind::Encode => self.generate_encode(ctx, &res.name, res.lifetime),
            CodeGenKind::Decode => self.generate_decode(ctx, &res.name, res.lifetime),
        };
        quote! {
            #ty
            #impls
        }
    }
    fn generate_encode(
        &self,
        ctx: &mut Context,
        name: &proc_macro2::Ident,
        lifetime: impl ToTokens,
    ) -> TokenStream {
        let prepare = self.members.iter().map(|Member { name, ty }| {
            let member = name.no_rename();
            ty.prepare(quote! { (&self.#member) }, ctx, true)
        });
        let encode = self.members.iter().map(|Member { name, ty }| {
            let member = name.no_rename();
            ty.encode(quote! { (&self.#member) }, ctx)
        });
        impl_encode(
            name,
            &lifetime,
            quote! {
                0 #(+ #prepare)*
            },
            quote! {
                #(#encode)*
            },
            quote!(),
        )
    }
    fn generate_decode(
        &self,
        ctx: &mut Context,
        name: &proc_macro2::Ident,
        lifetime: Option<TokenStream>,
    ) -> TokenStream {
        let decode = self.members.iter().map(|Member { name, ty }| {
            let member = name.no_rename();
            let expr = ty.decode(ctx);
            quote! {
                let #member = #expr;
            }
        });
        let members = self
            .members
            .iter()
            .map(|Member { name, .. }| name.no_rename());
        impl_decode(
            name,
            lifetime.as_ref(),
            quote! {
                {
                    #(#decode)*
                    #name {
                        #(#members,)*
                    }
                }
            },
        )
    }
}

impl Enum {
    fn generate(&self, ctx: &mut Context) -> TokenStream {
        let mut type_has_lifetime = false;
        let members = self.members.iter().map(|member| {
            let TypeGenResult {
                token_stream: ty,
                has_lifetime,
            } = match member.member.ty {
                Type::Integer(Integer { bit_width, .. }) if bit_width == 0 => TypeGenResult {
                    token_stream: quote!(),
                    has_lifetime: false,
                },
                _ => {
                    let res = member.member.ty.generate(ctx);
                    let ty = res.token_stream;
                    TypeGenResult {
                        token_stream: quote!((#ty)),
                        ..res
                    }
                }
            };
            type_has_lifetime |= has_lifetime;
            let cons = member.member.name.camel_case();
            quote! {
                #cons #ty,
            }
        });
        let members = quote! {
            #(#members)*
        };
        let lifetime = type_has_lifetime.then(|| {
            ctx.has_lifetime.insert(self.name.clone());
            quote!(<'a>)
        });
        let name = self.name.camel_case();
        let impls = match ctx.kind {
            CodeGenKind::Encode => {
                let prepare = self.members.iter().map(
                    |TaggedMember {
                         member: Member { name, ty },
                         ..
                     }| {
                        let cons = name.camel_case();
                        match ty {
                            &Type::Integer(Integer { bit_width, .. }) if bit_width == 0 => quote! {
                                Self::#cons => 0,
                            },
                            _ => {
                                let expr = ty.prepare(quote!(_x), ctx, true);
                                quote! {
                                    Self::#cons(_x) => #expr,
                                }
                            }
                        }
                    },
                );
                let encode = self.members.iter().map(
                    |&TaggedMember {
                         member: Member { ref name, ty },
                         discriminant,
                     }| {
                        let cons = name.camel_case();
                        let size = Literal::usize_unsuffixed(self.discriminant_size);
                        let discriminant = quote! {
                            unsafe {
                                w.push_unsigned(#discriminant, #size);
                            }
                        };
                        match ty {
                            Type::Integer(Integer { bit_width, .. }) if bit_width == 0 => quote! {
                                Self::#cons => #discriminant
                            },
                            _ => {
                                let expr = ty.encode(quote!(x), ctx);
                                quote! {
                                    Self::#cons(x) => {
                                        #discriminant
                                        #expr
                                    }
                                }
                            }
                        }
                    },
                );
                let discriminant = Literal::usize_unsuffixed(self.discriminant_size);
                impl_encode(
                    &name,
                    &lifetime,
                    quote! {
                        #discriminant + match self {
                            #(#prepare)*
                        }
                    },
                    quote! {
                        match self {
                            #(#encode)*
                        }
                    },
                    quote!(),
                )
            }
            CodeGenKind::Decode => {
                let decode = self.members.iter().map(
                    |&TaggedMember {
                         member: Member { ref name, ty },
                         discriminant,
                     }| {
                        let cons = name.camel_case();
                        match ty {
                            Type::Integer(Integer { bit_width, .. }) if bit_width == 0 => quote! {
                                #discriminant => Self::#cons,
                            },
                            _ => {
                                let expr = ty.decode(ctx);
                                quote! {
                                    #discriminant => Self::#cons(#expr),
                                }
                            }
                        }
                    },
                );
                let size = self.discriminant_size;
                impl_decode(
                    &name,
                    lifetime.as_ref(),
                    quote!({
                        let discriminant = ::nisba::const_try!(unsafe { r.next_unsigned(#size) });
                        match discriminant {
                            #(#decode)*
                            _ => return Err(::nisba::decode::Error::UnknownDiscriminant {
                                name: stringify!(#name),
                                value: discriminant as _
                            })
                        }
                    }),
                )
            }
        };
        quote! {
            pub enum #name #lifetime {
                #members
            }
            #impls
        }
    }
}

impl Dict {
    fn generate(&self, ctx: &mut Context) -> TokenStream {
        let res = StructGen::generate(
            &self.name,
            self.members.iter().map(|x| &x.member),
            ctx,
            true,
        );
        let ty = res.quote();
        let size = self.discriminant_size;
        let impls = match ctx.kind {
            CodeGenKind::Encode => {
                let discriminant_size = Literal::usize_unsuffixed(self.discriminant_size);
                let prepare = self.members.iter().map(
                    |TaggedMember {
                         member: Member { name, ty },
                         ..
                     }| {
                        let member = name.no_rename();
                        let prepare = ty.prepare(quote!(_x), ctx, true);
                        quote! {
                            if let Some(_x) = &self.#member {
                                #prepare
                            } else {
                                0
                            }
                        }
                    },
                );
                let bitmap = self.members.iter().map(
                    |&TaggedMember {
                         member: Member { ref name, .. },
                         discriminant,
                     }| {
                        let member = name.no_rename();
                        let shift = Literal::u64_unsuffixed(discriminant);
                        quote! {
                            if self.#member.is_some() {
                                1 << #shift
                            } else {
                                0
                            }
                        }
                    },
                );
                let encode = self.members.iter().map(
                    |&TaggedMember {
                         member: Member { ref name, ty },
                         ..
                     }| {
                        let member = name.no_rename();
                        match ty {
                            Type::Integer(Integer { bit_width, .. }) if bit_width == 0 => quote! {},
                            _ => {
                                let encode = ty.encode(quote!(x), ctx);
                                quote! {
                                    if let Some(x) = &self.#member {
                                        #encode
                                    }
                                }
                            }
                        }
                    },
                );
                impl_encode(
                    res.name,
                    res.lifetime,
                    quote! { #discriminant_size #( + #prepare)* },
                    quote! {
                        unsafe { w.push_unsigned(0 #( | #bitmap)*, #size); }
                        #(#encode)*
                    },
                    quote!(),
                )
            }
            CodeGenKind::Decode => {
                let decode = self.members.iter().map(
                    |&TaggedMember {
                         member: Member { ref name, ty },
                         discriminant,
                     }| {
                        let shift = Literal::u64_unsuffixed(discriminant);
                        let decode = ty.decode(ctx);
                        let member = name.no_rename();
                        quote! {
                            #member: if bitmap & 1 << #shift != 0 {
                                Some(#decode)
                            } else {
                                None
                            },
                        }
                    },
                );
                impl_decode(
                    &res.name,
                    res.lifetime.as_ref(),
                    quote!({
                        let bitmap = ::nisba::const_try!(unsafe { r.next_unsigned(#size) });
                        Self {
                            #(#decode)*
                        }
                    }),
                )
            }
        };
        quote! {
            #ty
            #impls
        }
    }
}

impl LenType {
    fn prepare(self, expr: TokenStream) -> TokenStream {
        match self {
            LenType::Fixed { size } => {
                let size = Literal::usize_unsuffixed(size as _);
                quote!(#size)
            }
            _ => {
                quote! {
                    ::nisba::encode::varint_calc_size_unsigned(#expr as _)
                }
            }
        }
    }
    fn encode(self, expr: TokenStream) -> TokenStream {
        match self {
            LenType::Fixed { size } => {
                let ty = Integer {
                    bit_width: size * 8,
                    signedness: Signedness::Unsigned,
                }
                .type_name();
                quote! {
                    unsafe { w.push_bytes(&(#expr as #ty).to_le_bytes()); }
                }
            }
            _ => quote! {
                unsafe { w.push_varint_unsigned(#expr as _) }
            },
        }
    }
    fn decode(self) -> TokenStream {
        match self {
            LenType::Fixed { size } => {
                let size = size as usize;
                quote!(::nisba::const_try!(unsafe { r.next_unsigned(#size) }) as usize)
            }
            LenType::V16 => quote!(::nisba::const_try!(r.next_varint_unsigned(16)) as usize),
            LenType::V32 => quote!(::nisba::const_try!(r.next_varint_unsigned(32)) as usize),
            LenType::V64 => quote!(::nisba::const_try!(r.next_varint_unsigned(64)) as usize),
        }
    }
    fn bit_width(self) -> u16 {
        match self {
            LenType::Fixed { size } => size * 8,
            LenType::V16 => 16,
            LenType::V32 => 32,
            LenType::V64 => 64,
        }
    }
    fn max_size(self) -> Literal {
        Literal::usize_unsuffixed((1usize << self.bit_width() - 1).wrapping_sub(1))
    }
}

fn fixed_sized_ty(ty: Type, ctx: &Context) -> TokenStream {
    match ty {
        Type::Integer(integer) => integer.type_name(),
        Type::Varint(_) => unreachable!(),
        Type::Definition(handle) => match ctx.schema.get_definition(handle) {
            Definition::Primitive(Primitive { name, .. })
            | Definition::Packed(Packed { name, .. })
            | Definition::Enum(Enum { name, .. })
            | Definition::Dict(Dict { name, .. }) => {
                let ty = name.camel_case();
                quote!(#ty)
            }
            Definition::Stream(_) | Definition::Vector(_) | Definition::Struct(_) => unreachable!(),
        },
    }
}

struct StructGen {
    name: proc_macro2::Ident,
    lifetime: Option<TokenStream>,
    fields: TokenStream,
}

impl StructGen {
    fn generate<'a>(
        name: &Ident,
        members: impl Iterator<Item = &'a Member>,
        ctx: &mut Context,
        option: bool,
    ) -> Self {
        let mut type_has_lifetime = false;
        let fields = members.map(|Member { name, ty }| {
            let TypeGenResult {
                token_stream,
                has_lifetime,
            } = generate_field_option(name, ty, ctx, option);
            type_has_lifetime |= has_lifetime;
            token_stream
        });
        let fields = quote! {
            #(#fields)*
        };
        let lifetime = type_has_lifetime.then(|| {
            ctx.has_lifetime.insert(name.clone());
            quote!(<'a>)
        });
        let name = name.camel_case();
        Self {
            name,
            lifetime,
            fields,
        }
    }
    fn quote(&self) -> TokenStream {
        let Self {
            name,
            lifetime,
            fields,
        } = self;
        quote! {
            pub struct #name #lifetime {
                #fields
            }
        }
    }
}

fn impl_encode(
    name: impl ToTokens,
    lifetime: impl ToTokens,
    prepare: impl ToTokens,
    encode: impl ToTokens,
    con: impl ToTokens,
) -> TokenStream {
    quote! {
        impl #lifetime #name #lifetime {
            pub #con fn prepare(&self) -> ::nisba::encode::Result<usize> {
                Ok(#prepare)
            }
            pub #con unsafe fn encode(&self, w: &mut ::nisba::encode::Encoder) {
                #encode
            }
        }
        unsafe impl #lifetime ::nisba::encode::Encode for #name #lifetime {
            fn prepare(&self) -> ::nisba::encode::Result<usize> {
                self.prepare()
            }
            unsafe fn encode(&self, w: &mut ::nisba::encode::Encoder) {
                unsafe { self.encode(w) }
            }
        }
    }
}

fn impl_decode(
    name: impl ToTokens,
    lifetime: Option<&TokenStream>,
    decode: impl ToTokens,
) -> TokenStream {
    if let Some(lifetime) = lifetime {
        quote! {
            impl #lifetime #name #lifetime {
                pub const fn decode(r: &mut ::nisba::decode::Decoder #lifetime) -> ::nisba::decode::Result<Self> {
                    Ok(#decode)
                }
            }
            unsafe impl #lifetime ::nisba::decode::Decode #lifetime for #name #lifetime {
                fn decode(r: &mut ::nisba::decode::Decoder #lifetime) -> ::nisba::decode::Result<Self> {
                    Self::decode(r)
                }
            }
        }
    } else {
        quote! {
            impl #name {
                pub const fn decode(r: &mut ::nisba::decode::Decoder) -> ::nisba::decode::Result<Self> {
                    Ok(#decode)
                }
            }
            unsafe impl ::nisba::decode::Decode<'_> for #name {
                fn decode(r: &mut ::nisba::decode::Decoder) -> ::nisba::decode::Result<Self> {
                    Self::decode(r)
                }
            }
        }
    }
}

fn generate_field_option(name: &Ident, ty: &Type, ctx: &Context, option: bool) -> TypeGenResult {
    let name = name.no_rename();
    let TypeGenResult {
        token_stream: ty,
        has_lifetime,
    } = ty.generate(ctx);
    let token_stream = if option {
        quote! {
            pub #name: Option<#ty>,
        }
    } else {
        quote! {
            pub #name: #ty,
        }
    };
    TypeGenResult {
        token_stream,
        has_lifetime,
    }
}

struct TypeGenResult {
    token_stream: TokenStream,
    has_lifetime: bool,
}

impl Integer {
    fn rust_integer(self) -> proc_macro2::Ident {
        let Self {
            bit_width,
            signedness,
        } = self;
        let signedness = match signedness {
            Signedness::Signed => 'i',
            Signedness::Unsigned => 'u',
        };
        proc_macro2::Ident::new(&format!("{signedness}{bit_width}"), Span::call_site())
    }
    fn type_name(self) -> TokenStream {
        match self.bit_width {
            0 => quote!(()),
            8 | 16 | 32 | 64 | 128 => self.rust_integer().to_token_stream(),
            _ => {
                let size = self.bit_width as usize / 8;
                quote!([u8; #size])
            }
        }
    }
    fn encode(self, expr: impl ToTokens) -> TokenStream {
        match self.bit_width {
            0 => quote!(),
            8 | 16 | 32 | 64 | 128 => quote! {
                unsafe { w.push_bytes(&#expr.to_le_bytes()); }
            },
            _ => quote! {
                unsafe { w.push_bytes(#expr); }
            },
        }
    }
    fn decode(self) -> TokenStream {
        match self.bit_width {
            0 => quote!(()),
            8 | 16 | 32 | 64 | 128 => {
                let ty = self.type_name();
                quote! {
                    #ty::from_le_bytes(::nisba::const_try!(r.next_u8_array()))
                }
            }
            _ => {
                let size = self.bit_width as usize / 8;
                quote! {
                    ::nisba::const_try!(r.next_u8_array::<#size>())
                }
            }
        }
    }
    fn decode_varint(self) -> TokenStream {
        let Self {
            bit_width,
            signedness,
        } = self;
        let ty = self.rust_integer();
        let bit_width = Literal::usize_unsuffixed(bit_width as _);
        match signedness {
            Signedness::Signed => quote! {
                ::nisba::const_try!(r.next_varint_signed(#bit_width)) as #ty
            },
            Signedness::Unsigned => quote! {
                ::nisba::const_try!(r.next_varint_unsigned(#bit_width)) as #ty
            },
        }
    }
}

impl Type {
    fn generate(self, ctx: &Context) -> TypeGenResult {
        match self {
            Type::Integer(integer) | Type::Varint(integer) => TypeGenResult {
                token_stream: integer.type_name(),
                has_lifetime: false,
            },
            Type::Definition(handle) => match ctx.schema.get_definition(handle) {
                Definition::Primitive(Primitive { name, .. }) => {
                    let ty = name.no_rename();
                    TypeGenResult {
                        token_stream: quote!(#ty),
                        has_lifetime: false,
                    }
                }
                Definition::Packed(Packed { name, .. })
                | Definition::Struct(Struct { name, .. })
                | Definition::Enum(Enum { name, .. })
                | Definition::Dict(Dict { name, .. }) => {
                    let ty = name.camel_case();
                    if ctx.has_lifetime.contains(name) {
                        TypeGenResult {
                            token_stream: quote!(#ty<'a>),
                            has_lifetime: true,
                        }
                    } else {
                        TypeGenResult {
                            token_stream: quote!(#ty),
                            has_lifetime: false,
                        }
                    }
                }
                Definition::Vector(Vector { element_type, .. }) => {
                    let TypeGenResult {
                        token_stream: ty,
                        has_lifetime,
                    } = element_type.generate(ctx);
                    match (ctx.kind, has_lifetime) {
                        (CodeGenKind::Encode, false) => TypeGenResult {
                            token_stream: quote!(Vec<#ty>),
                            has_lifetime,
                        },
                        (CodeGenKind::Encode, true) => TypeGenResult {
                            token_stream: quote!(Vec<#ty<'a>>),
                            has_lifetime,
                        },
                        (CodeGenKind::Decode, _) => TypeGenResult {
                            token_stream: quote!(::nisba::decode::Vector<'a, #ty>),
                            has_lifetime: true,
                        },
                    }
                }
                Definition::Stream(Stream { element_type, .. }) => {
                    let TypeGenResult {
                        token_stream: ty,
                        has_lifetime,
                    } = element_type.generate(ctx);
                    match (ctx.kind, has_lifetime) {
                        (CodeGenKind::Encode, false) => TypeGenResult {
                            token_stream: quote!(Vec<#ty>),
                            has_lifetime,
                        },
                        (CodeGenKind::Encode, true) => TypeGenResult {
                            token_stream: quote!(Vec<#ty<'a>>),
                            has_lifetime,
                        },
                        (CodeGenKind::Decode, _) => TypeGenResult {
                            token_stream: quote!(::nisba::decode::Stream<'a, #ty>),
                            has_lifetime: true,
                        },
                    }
                }
            },
        }
    }
    fn prepare(self, expr: TokenStream, ctx: &Context, validate: bool) -> TokenStream {
        match self {
            Type::Integer(integer) => {
                let ty = integer.type_name();
                quote! { ::core::mem::size_of::<#ty>() }
            }
            Type::Varint(Integer { signedness, .. }) => match signedness {
                Signedness::Unsigned => quote! {
                    ::nisba::encode::varint_calc_size_unsigned(*#expr as _)
                },
                Signedness::Signed => quote! {
                    ::nisba::encode::varint_calc_size_signed(*#expr as _)
                },
            },
            Type::Definition(handle) => match ctx.schema.get_definition(handle) {
                Definition::Primitive(primitive) => {
                    let name = primitive.name.no_rename();
                    quote! {
                        ::core::mem::size_of::<#name>()
                    }
                }
                &Definition::Vector(Vector {
                    len_type,
                    element_type,
                }) => {
                    let max_size = len_type.max_size();
                    let len = len_type.prepare(quote! { #expr.len() });
                    let elem_ty = fixed_sized_ty(element_type, ctx);
                    let elems = quote! {
                        ::core::mem::size_of::<#elem_ty>() * #expr.len()
                    };
                    if validate {
                        let validate = quote! {
                            if #expr.len() > #max_size {
                                return Err(::nisba::encode::Error::ContainerLengthOverflow)
                            }
                        };
                        quote!({
                            #validate
                            #len + #elems
                        })
                    } else {
                        quote!(#len + #elems)
                    }
                }
                Definition::Stream(Stream {
                    len_type,
                    element_type,
                }) => {
                    let max_size = len_type.max_size();
                    let len = len_type.prepare(quote!(byte_count));
                    let prepare = element_type.prepare(quote!(x), ctx, false);
                    quote!({
                        let mut byte_count = 0;
                        let mut iter = #expr.iter();
                        while let Some(x) = iter.next() {
                            byte_count += #prepare;
                            if byte_count > #max_size {
                                return Err(::nisba::encode::Error::ContainerLengthOverflow);
                            }
                        }
                        #len + byte_count
                    })
                }

                Definition::Packed(_)
                | Definition::Struct(_)
                | Definition::Enum(_)
                | Definition::Dict(_) => match validate {
                    true => quote! { ::nisba::const_try!(#expr.prepare()) },
                    false => quote! { unsafe { #expr.prepare().unwrap_unchecked() } },
                },
            },
        }
    }

    fn encode(self, expr: TokenStream, ctx: &Context) -> TokenStream {
        match self {
            Type::Integer(integer) => integer.encode(expr),
            Type::Varint(_) => quote! {
                unsafe { w.push_varint_unsigned(*#expr as _); }
            },
            Type::Definition(handle) => match ctx.schema.get_definition(handle) {
                Definition::Primitive(Primitive { name, .. }) => {
                    let name = name.no_rename();
                    quote! {
                        w.push_bytes(::core::slice::from_raw_parts(
                            #expr as _,
                            ::core::mem::size_of::<#name>()
                        ))
                    }
                }
                &Definition::Vector(Vector {
                    len_type,
                    element_type,
                }) => {
                    let len = len_type.encode(quote!(#expr.len()));
                    let elems = element_type.encode(quote!(x), ctx);
                    quote! {
                        #len
                        let mut iter = #expr.iter();
                        while let Some(x) = iter.next() {
                            #elems
                        }
                    }
                }
                &Definition::Stream(Stream {
                    len_type,
                    element_type,
                }) => {
                    let len = len_type.encode(quote!(byte_count));
                    let count = element_type.prepare(quote!(x), ctx, false);
                    let elems = element_type.encode(quote!(x), ctx);
                    quote! {
                        let mut byte_count = 0;
                        let mut iter = #expr.iter();
                        while let Some(x) = iter.next() {
                            byte_count += #count;
                        }
                        #len
                        while let Some(x) = iter.next() {
                            #elems
                        }
                    }
                }

                Definition::Packed(_)
                | Definition::Struct(_)
                | Definition::Enum(_)
                | Definition::Dict(_) => quote! {
                    unsafe { #expr.encode(w); }
                },
            },
        }
    }

    fn decode(self, ctx: &Context) -> TokenStream {
        match self {
            Type::Integer(integer) => {
                if integer.bit_width == 0 {
                    quote!(())
                } else {
                    integer.decode()
                }
            }
            Type::Varint(integer) => integer.decode_varint(),
            Type::Definition(handle) => match ctx.schema.get_definition(handle) {
                Definition::Primitive(Primitive { name, .. }) => decode_primitive(name.no_rename()),
                &Definition::Vector(Vector {
                    len_type,
                    element_type,
                }) => {
                    let len = len_type.decode();
                    let ty = fixed_sized_ty(element_type, ctx);
                    quote! ({
                        let len = #len;
                        ::nisba::decode::Vector::new(::nisba::const_try!(r.next_bytes(len * ::core::mem::size_of::<#ty>())))
                    })
                }
                Definition::Stream(Stream { len_type, .. }) => {
                    let len = len_type.decode();
                    quote! ({
                        let len = #len;
                        ::nisba::decode::Stream::new(::nisba::const_try!(r.next_bytes(len)))
                    })
                }
                Definition::Packed(Packed { name, .. })
                | Definition::Struct(Struct { name, .. })
                | Definition::Enum(Enum { name, .. })
                | Definition::Dict(Dict { name, .. }) => {
                    let ty = name.camel_case();
                    quote! {
                        ::nisba::const_try!(#ty::decode(r))
                    }
                }
            },
        }
    }
}

fn decode_primitive(ty: impl ToTokens) -> TokenStream {
    quote! {
        unsafe {
            let mut res = ::core::mem::MaybeUninit::<#ty>::uninit();
            ::nisba::const_try!(r.next_bytes(::core::mem::size_of::<#ty>()))
                .as_ptr()
                .copy_to_nonoverlapping(res.as_mut_ptr().cast(), ::core::mem::size_of::<#ty>());
            res.assume_init()
        }
    }
}
