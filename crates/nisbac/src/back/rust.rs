use heck::ToUpperCamelCase;
use proc_macro2::{Literal, Span, TokenStream};
use quote::{ToTokens, quote};

use crate::{
    Ident,
    back::CodeGenKind,
    schema::{validator::BitWidth, *},
};

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
#[derive(Default)]
pub struct Config {}

struct Context<'a> {
    schema: &'a Validated,
    has_lifetime: Box<[bool]>,
    #[allow(dead_code)]
    config: Config,
    kind: CodeGenKind,
}

impl Vector {
    fn zero_copy(&self, bit_width: &[BitWidth]) -> bool {
        self.elem_ty.is_fixed_size(bit_width)
    }
}

impl Context<'_> {
    fn calc_lifetime(&mut self) {
        if self.kind == CodeGenKind::Encode {
            return;
        }
        self.schema
            .schema
            .definitions
            .iter()
            .enumerate()
            .filter_map(|(idx, def)| match def {
                Definition::Stream(_) => Some(idx),
                Definition::Vector(v) if v.zero_copy(&self.schema.bit_width) => Some(idx),
                _ => None,
            })
            .map(Handle)
            .for_each(|x| self.mark_lifetime(x));
    }
    fn mark_lifetime(&mut self, root: Handle) {
        if !self.has_lifetime[root.0] {
            self.has_lifetime[root.0] = true;
            self.schema.referrers[root.0]
                .iter()
                .copied()
                .for_each(|x| self.mark_lifetime(x));
        }
    }
}

pub fn generate(schema: &Validated, kind: CodeGenKind, config: Config) -> TokenStream {
    let mut ctx = Context {
        schema,
        has_lifetime: vec![false; schema.schema.definitions.len()].into(),
        config,
        kind,
    };
    ctx.calc_lifetime();
    let definitions = schema
        .schema
        .definitions
        .iter()
        .enumerate()
        .map(|(idx, definition)| {
            let has_lifetime = ctx.has_lifetime[idx];
            match definition {
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
                Definition::Packed(x) => x.generate(&mut ctx),
                Definition::Struct(x) => x.generate(&mut ctx, has_lifetime),
                Definition::Enum(x) => x.generate(&mut ctx, has_lifetime),
                Definition::Dict(x) => x.generate(&mut ctx, has_lifetime),
                Definition::Extern(_) => quote!(),
            }
        });
    quote! {
        #(#definitions)*
    }
}

impl Packed {
    fn is_bitfield(&self, ctx: &Context) -> bool {
        self.members
            .iter()
            .any(|member| member.ty.fixed_bit_width(&ctx.schema.bit_width).unwrap() % 8 != 0)
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
            .map(|member| member.ty.fixed_bit_width(&ctx.schema.bit_width).unwrap())
            .sum();
        let byte_size = Literal::usize_unsuffixed(bit_size / 8);
        let name = self.name.camel_case();
        let impls = match ctx.kind {
            CodeGenKind::Encode => impl_encode(
                &name,
                quote!(),
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
            #[derive(Debug)]
            pub struct #name(pub [u8; #byte_size]);
            #impls
        }
    }
    fn generate_struct(&self, ctx: &mut Context) -> TokenStream {
        let name = self.name.camel_case();
        let body = StructGen::generate(&self.name, self.members.iter(), ctx, false).quote(quote!());
        let impls = match ctx.kind {
            CodeGenKind::Encode => impl_encode(
                &name,
                quote!(),
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
    fn generate(&self, ctx: &mut Context, has_lifetime: bool) -> TokenStream {
        let lifetime = has_lifetime.then(|| quote!(<'a>));
        let res = StructGen::generate(&self.name, self.members.iter(), ctx, false);
        let ty = res.quote(&lifetime);
        let impls = match ctx.kind {
            CodeGenKind::Encode => self.generate_encode(ctx, &res.name, None),
            CodeGenKind::Decode => self.generate_decode(ctx, &res.name, lifetime),
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
        lifetime: Option<TokenStream>,
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
    fn generate(&self, ctx: &mut Context, has_lifetime: bool) -> TokenStream {
        let members = self.members.iter().map(|IndexedMember { member, .. }| {
            let ty = match member.ty {
                Type::Integer { bit_width: 0, .. } => quote!(),
                _ => {
                    let ty = member.ty.generate(ctx, false);
                    quote!((#ty))
                }
            };
            let cons = member.name.camel_case();
            quote! {
                #cons #ty,
            }
        });
        let members = quote! {
            #(#members)*
        };
        let name = self.name.camel_case();
        let lifetime = has_lifetime.then(|| quote!(<'a>));
        let impls = match ctx.kind {
            CodeGenKind::Encode => {
                let prepare = self.members.iter().map(
                    |IndexedMember {
                         member: Member { name, ty },
                         ..
                     }| {
                        let cons = name.camel_case();
                        match ty {
                            &Type::Integer { bit_width: 0, .. } => quote! {
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
                    |&IndexedMember {
                         member: Member { ref name, ty },
                         index,
                     }| {
                        let cons = name.camel_case();
                        let size = Literal::usize_unsuffixed(self.index_ty as _);
                        let index = quote! {
                            unsafe {
                                w.push_unsigned(#index, #size);
                            }
                        };
                        match ty {
                            Type::Integer { bit_width: 0, .. } => quote! {
                                Self::#cons => #index
                            },
                            _ => {
                                let expr = ty.encode(quote!(x), ctx);
                                quote! {
                                    Self::#cons(x) => {
                                        #index
                                        #expr
                                    }
                                }
                            }
                        }
                    },
                );
                let discriminant = Literal::usize_unsuffixed(self.index_ty as _);
                impl_encode(
                    &name,
                    quote!(),
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
                    |&IndexedMember {
                         member: Member { ref name, ty },
                         index,
                     }| {
                        let cons = name.camel_case();
                        match ty {
                            Type::Integer { bit_width: 0, .. } => quote! {
                                #index => Self::#cons,
                            },
                            _ => {
                                let expr = ty.decode(ctx);
                                quote! {
                                    #index => Self::#cons(#expr),
                                }
                            }
                        }
                    },
                );
                let size = self.index_ty as usize;
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
            #[derive(Debug)]
            pub enum #name #lifetime {
                #members
            }
            #impls
        }
    }
}

impl Dict {
    fn generate(&self, ctx: &mut Context, has_lifetime: bool) -> TokenStream {
        let res = StructGen::generate(
            &self.name,
            self.members.iter().map(|x| &x.member),
            ctx,
            true,
        );
        let lifetime = has_lifetime.then(|| quote!(<'a>));
        let ty = res.quote(&lifetime);
        let size = self.index_ty as usize;
        let impls = match ctx.kind {
            CodeGenKind::Encode => {
                let discriminant_size = Literal::usize_unsuffixed(self.index_ty as _);
                let prepare = self.members.iter().map(
                    |IndexedMember {
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
                    |&IndexedMember {
                         member: Member { ref name, .. },
                         index,
                     }| {
                        let member = name.no_rename();
                        let shift = Literal::u64_unsuffixed(index);
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
                    |&IndexedMember {
                         member: Member { ref name, ty },
                         ..
                     }| {
                        let member = name.no_rename();
                        match ty {
                            Type::Integer { bit_width: 0, .. } => quote! {},
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
                    quote!(),
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
                    |&IndexedMember {
                         member: Member { ref name, ty },
                         index,
                     }| {
                        let shift = Literal::u64_unsuffixed(index);
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
                    lifetime.as_ref(),
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
    fn type_name(self) -> TokenStream {
        match self {
            LenType::V16 => quote!(::nisba::wrappers::Varint::<u16>),
            LenType::V32 => quote!(::nisba::wrappers::Varint::<u32>),
            LenType::V64 => quote!(::nisba::wrappers::Varint::<u64>),
            _ => {
                let n = Literal::usize_unsuffixed(self as _);
                quote!(::nisba::wrappers::Integer::<#n>)
            }
        }
    }
    fn prepare(self, expr: TokenStream) -> TokenStream {
        match self {
            LenType::V16 | LenType::V32 | LenType::V64 => quote! {
                ::nisba::encode::varint_calc_size_unsigned(#expr as _)
            },
            _ => {
                let size = Literal::usize_unsuffixed(self as _);
                quote!(#size)
            }
        }
    }
    fn encode(self, expr: TokenStream) -> TokenStream {
        match self {
            LenType::V16 | LenType::V32 | LenType::V64 => quote! {
                unsafe { w.push_varint_unsigned(#expr as _) }
            },
            _ => {
                let ty = Integer {
                    bit_width: self.fixed_size() * 8,
                    signedness: Signedness::Unsigned,
                }
                .type_name();
                quote! {
                    unsafe { w.push_bytes(&(#expr as #ty).to_le_bytes()); }
                }
            }
        }
    }
    fn decode(self) -> TokenStream {
        match self {
            LenType::V16 | LenType::V32 | LenType::V64 => {
                let bit_width = Literal::usize_unsuffixed(self.varint_size() as usize * 8);
                quote! {
                    unsafe { ::nisba::const_try!(r.next_varint_unsigned(#bit_width)) }
                }
            }
            _ => {
                let bit_width = Literal::usize_unsuffixed(self.fixed_size() as usize * 8);
                quote! {
                    unsafe { ::nisba::const_try!(r.next_unsigned(#bit_width)) }
                }
            }
        }
    }
    fn bit_width(self) -> u16 {
        match self {
            LenType::V16 => 16,
            LenType::V32 => 32,
            LenType::V64 => 64,
            _ => self.fixed_size() * 8,
        }
    }
    fn max_size(self) -> Literal {
        Literal::usize_unsuffixed((1usize << (self.bit_width() - 1)).wrapping_sub(1))
    }
}

struct StructGen {
    name: proc_macro2::Ident,
    fields: TokenStream,
}

impl StructGen {
    fn generate<'a>(
        name: &Ident,
        members: impl Iterator<Item = &'a Member>,
        ctx: &mut Context,
        option: bool,
    ) -> Self {
        let fields =
            members.map(|Member { name, ty }| generate_field_option(name, ty, ctx, option));
        let fields = quote! {
            #(#fields)*
        };
        let name = name.camel_case();
        Self { name, fields }
    }
    fn quote(&self, lifetime: impl ToTokens) -> TokenStream {
        let Self { name, fields } = self;
        quote! {
            #[derive(Debug)]
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
            #[inline]
            pub #con fn prepare(&self) -> ::nisba::encode::Result<usize> {
                Ok(#prepare)
            }
            pub #con unsafe fn encode(&self, w: &mut ::nisba::encode::Encoder) {
                #encode
            }
        }
        unsafe impl #lifetime ::nisba::encode::Encode for #name #lifetime {
            #[inline]
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
            unsafe impl #lifetime ::nisba::decode::Decode #lifetime for #name #lifetime {
                fn decode(r: &mut ::nisba::decode::Decoder #lifetime) -> ::nisba::decode::Result<Self> {
                    Ok(#decode)
                }
            }
        }
    } else {
        quote! {
            unsafe impl ::nisba::decode::Decode<'_> for #name {
                fn decode(r: &mut ::nisba::decode::Decoder) -> ::nisba::decode::Result<Self> {
                    Ok(#decode)
                }
            }
        }
    }
}

fn generate_field_option(name: &Ident, ty: &Type, ctx: &Context, option: bool) -> TokenStream {
    let name = name.no_rename();
    let ty = ty.generate(ctx, false);
    if option {
        quote! {
            pub #name: Option<#ty>,
        }
    } else {
        quote! {
            pub #name: #ty,
        }
    }
}

struct Integer {
    signedness: Signedness,
    bit_width: u16,
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
    fn wrapper(self) -> TokenStream {
        match self.bit_width {
            0 => quote!(::nisba::wrappers::Primitive<()>),
            8 | 16 | 32 | 64 | 128 => {
                let ty = self.rust_integer().to_token_stream();
                quote!(::nisba::wrappers::Primitive<#ty>)
            }
            _ => {
                let size = self.bit_width as usize / 8;
                quote!(::nisba::wrappers::Integer<#size>)
            }
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
}

fn encode_integer(bit_width: u16, expr: impl ToTokens) -> TokenStream {
    match bit_width {
        0 => quote!(),
        8 | 16 | 32 | 64 | 128 => quote! {
            unsafe { w.push_bytes(&#expr.to_le_bytes()); }
        },
        _ => quote! {
            unsafe { w.push_bytes(#expr); }
        },
    }
}

fn varint_type_name(size: VarintSize, signedness: Signedness) -> TokenStream {
    match (signedness, size) {
        (Signedness::Signed, VarintSize::V16) => quote!(i16),
        (Signedness::Signed, VarintSize::V32) => quote!(i32),
        (Signedness::Signed, VarintSize::V64) => quote!(i64),
        (Signedness::Unsigned, VarintSize::V16) => quote!(u16),
        (Signedness::Unsigned, VarintSize::V32) => quote!(u32),
        (Signedness::Unsigned, VarintSize::V64) => quote!(u64),
    }
}

fn decode_varint(size: VarintSize, signedness: Signedness) -> TokenStream {
    let ty = varint_type_name(size, signedness);
    let bit_width = Literal::usize_unsuffixed(size as usize * 8);
    match signedness {
        Signedness::Signed => quote! {
            ::nisba::const_try!(unsafe { r.next_varint_signed(#bit_width) }) as #ty
        },
        Signedness::Unsigned => quote! {
            ::nisba::const_try!(unsafe { r.next_varint_unsigned(#bit_width) }) as #ty
        },
    }
}

#[derive(Clone, Copy)]
enum Sequence {
    Vector,
    Stream,
}

impl Sequence {
    fn name_encode(elem_ty: Type, ctx: &Context) -> TokenStream {
        let ty = elem_ty.generate(ctx, false);
        quote!(Vec<#ty>)
    }
    fn name_decode(self, elem_ty: Type, len_ty: LenType, ctx: &Context) -> TokenStream {
        let len = len_ty.type_name();
        match self {
            Sequence::Vector => {
                if elem_ty.is_fixed_size(&ctx.schema.bit_width) {
                    let ty = elem_ty.generate(ctx, true);
                    quote!(::nisba::decode::Slice::<'a, #ty, #len>)
                } else {
                    let ty = elem_ty.generate(ctx, false);
                    quote!(Box::<[#ty]>)
                }
            }
            Sequence::Stream => {
                let ty = elem_ty.generate(ctx, true);
                quote!(::nisba::decode::Stream::<'a, #ty, #len>)
            }
        }
    }
    fn decode(self, elem_ty: Type, len_ty: LenType, ctx: &Context) -> TokenStream {
        let ty = self.name_decode(elem_ty, len_ty, ctx);
        let lazy = match self {
            Sequence::Vector => elem_ty.is_fixed_size(&ctx.schema.bit_width),
            Sequence::Stream => true,
        };
        if lazy {
            quote! {
                ::nisba::const_try!(<#ty as ::nisba::decode::Decode>::decode(r))
            }
        } else {
            let len = len_ty.decode();
            let elem = elem_ty.decode(ctx);
            quote! {{
                let len = #len as usize;
                let mut items = Box::new_uninit_slice(len);
                for idx in 0..len {
                    unsafe {
                        items.get_unchecked_mut(idx).write(#elem);
                    }
                }
                unsafe { items.assume_init() }
            }}
        }
    }
    fn type_name(self, elem_ty: Type, len_ty: LenType, ctx: &Context) -> TokenStream {
        match ctx.kind {
            CodeGenKind::Encode => Self::name_encode(elem_ty, ctx),
            CodeGenKind::Decode => self.name_decode(elem_ty, len_ty, ctx),
        }
    }
}

impl Type {
    fn fixed_bit_width(self, bit_width: &[BitWidth]) -> Option<usize> {
        match self {
            Type::Allocated(handle) => match bit_width[handle.0] {
                BitWidth::Fixed(x) => Some(x),
                BitWidth::Variable => None,
            },
            Type::Integer { bit_width, .. } => Some(bit_width as _),
            Type::Varint { .. } => None,
        }
    }
    fn is_fixed_size(self, bit_width: &[BitWidth]) -> bool {
        self.fixed_bit_width(bit_width).is_some()
    }
    fn fixed_size(self, bit_width: &[BitWidth]) -> Option<usize> {
        self.fixed_bit_width(bit_width).map(|x| x / 8)
    }
    fn generate(self, ctx: &Context, wrapper: bool) -> TokenStream {
        match self {
            Type::Integer {
                signedness,
                bit_width,
            } => {
                let int = Integer {
                    signedness,
                    bit_width,
                };
                match wrapper {
                    false => int.type_name(),
                    true => int.wrapper(),
                }
            }
            Type::Varint { signedness, size } => {
                let ty = varint_type_name(size, signedness);
                match wrapper {
                    false => ty,
                    true => quote!(::nisba::wrappers::Varint<#ty>),
                }
            }
            Type::Allocated(handle) => {
                let has_lifetime = ctx.has_lifetime[handle.0] && ctx.kind == CodeGenKind::Decode;
                match &ctx.schema.schema.definitions[handle.0] {
                    Definition::Primitive(Primitive { name, .. }) | Definition::Extern(name) => {
                        // TODO: support external types with lifetime
                        let ty = name.no_rename();
                        quote!(#ty)
                    }
                    Definition::Packed(Packed { name, .. })
                    | Definition::Struct(Struct { name, .. })
                    | Definition::Enum(Enum { name, .. })
                    | Definition::Dict(Dict { name, .. }) => {
                        let ty = name.camel_case();
                        if has_lifetime {
                            quote!(#ty<'a>)
                        } else {
                            quote!(#ty)
                        }
                    }
                    &Definition::Vector(Vector { elem_ty, len_ty }) => {
                        Sequence::Vector.type_name(elem_ty, len_ty, ctx)
                    }
                    &Definition::Stream(Stream { elem_ty, len_ty }) => {
                        Sequence::Stream.type_name(elem_ty, len_ty, ctx)
                    }
                }
            }
        }
    }
    fn prepare(self, expr: TokenStream, ctx: &Context, validate: bool) -> TokenStream {
        match self {
            Type::Integer {
                signedness,
                bit_width,
            } => {
                let ty = Integer {
                    signedness,
                    bit_width,
                }
                .type_name();
                quote! { ::core::mem::size_of::<#ty>() }
            }
            Type::Varint { signedness, .. } => match signedness {
                Signedness::Unsigned => quote! {
                    ::nisba::encode::varint_calc_size_unsigned(*#expr as _)
                },
                Signedness::Signed => quote! {
                    ::nisba::encode::varint_calc_size_signed(*#expr as _)
                },
            },
            Type::Allocated(handle) => match &ctx.schema.schema.definitions[handle.0] {
                Definition::Primitive(primitive) => {
                    let name = primitive.name.no_rename();
                    quote! {
                        ::core::mem::size_of::<#name>()
                    }
                }
                &Definition::Vector(Vector { len_ty, elem_ty }) => {
                    let max_size = len_ty.max_size();
                    let len = len_ty.prepare(quote! { #expr.len() });
                    let elems = match elem_ty.fixed_size(&ctx.schema.bit_width) {
                        Some(x) => quote! { #x * #expr.len() },
                        None => {
                            let prepare = elem_ty.prepare(quote!(x), ctx, false);
                            quote!({
                                let mut byte_count = 0;
                                let mut iter = #expr.iter();
                                while let Some(x) = iter.next() {
                                    byte_count += #prepare;
                                }
                                byte_count
                            })
                        }
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
                Definition::Stream(Stream { len_ty, elem_ty }) => {
                    let max_size = len_ty.max_size();
                    let len = len_ty.prepare(quote!(byte_count));
                    let prepare = elem_ty.prepare(quote!(x), ctx, false);
                    let validate = validate.then_some(quote! {
                        if byte_count > #max_size {
                            return Err(::nisba::encode::Error::ContainerLengthOverflow);
                        }
                    });
                    quote!({
                        let mut byte_count = 0;
                        let mut iter = #expr.iter();
                        while let Some(x) = iter.next() {
                            byte_count += #prepare;
                        }
                        #validate
                        #len + byte_count
                    })
                }
                Definition::Packed(_)
                | Definition::Struct(_)
                | Definition::Enum(_)
                | Definition::Dict(_)
                | Definition::Extern(_) => match validate {
                    true => quote! { ::nisba::const_try!(#expr.prepare()) },
                    false => quote! { unsafe { #expr.prepare().unwrap_unchecked() } },
                },
            },
        }
    }
    fn encode(self, expr: TokenStream, ctx: &Context) -> TokenStream {
        match self {
            Type::Integer { bit_width, .. } => encode_integer(bit_width, expr),
            Type::Varint { signedness, .. } => match signedness {
                Signedness::Signed => quote! {
                    unsafe { w.push_varint_signed(*#expr as _); }
                },
                Signedness::Unsigned => quote! {
                    unsafe { w.push_varint_unsigned(*#expr as _); }
                },
            },
            Type::Allocated(handle) => match &ctx.schema.schema.definitions[handle.0] {
                Definition::Primitive(Primitive { name, .. }) => {
                    let name = name.no_rename();
                    quote! {
                        unsafe {
                            let expr = #expr;
                            w.push_bytes(::core::slice::from_raw_parts(
                                (&raw const *expr).cast(),
                                ::core::mem::size_of::<#name>()
                            ))
                        }
                    }
                }
                &Definition::Vector(Vector { len_ty, elem_ty }) => {
                    let len = len_ty.encode(quote!(#expr.len()));
                    let elems = elem_ty.encode(quote!(x), ctx);
                    quote! {
                        #len
                        let mut iter = #expr.iter();
                        while let Some(x) = iter.next() {
                            #elems
                        }
                    }
                }
                &Definition::Stream(Stream { len_ty, elem_ty }) => {
                    let len = len_ty.encode(quote!(byte_count));
                    let count = elem_ty.prepare(quote!(x), ctx, false);
                    let elems = elem_ty.encode(quote!(x), ctx);
                    quote! {
                        let mut byte_count = 0;
                        let mut iter = #expr.iter();
                        while let Some(x) = iter.next() {
                            byte_count += #count;
                        }
                        #len
                        let mut iter = #expr.iter();
                        while let Some(x) = iter.next() {
                            #elems
                        }
                    }
                }

                Definition::Packed(_)
                | Definition::Struct(_)
                | Definition::Enum(_)
                | Definition::Dict(_)
                | Definition::Extern(_) => quote! {
                    unsafe { #expr.encode(w); }
                },
            },
        }
    }

    fn decode(self, ctx: &Context) -> TokenStream {
        match self {
            Type::Integer {
                signedness,
                bit_width,
            } => {
                if bit_width == 0 {
                    quote!(())
                } else {
                    Integer {
                        signedness,
                        bit_width,
                    }
                    .decode()
                }
            }
            Type::Varint { signedness, size } => decode_varint(size, signedness),
            Type::Allocated(handle) => match &ctx.schema.schema.definitions[handle.0] {
                Definition::Primitive(Primitive { name, .. }) => decode_primitive(name.no_rename()),
                &Definition::Vector(Vector { len_ty, elem_ty }) => {
                    Sequence::Vector.decode(elem_ty, len_ty, ctx)
                }
                &Definition::Stream(Stream { len_ty, elem_ty }) => {
                    Sequence::Stream.decode(elem_ty, len_ty, ctx)
                }
                Definition::Packed(Packed { name, .. })
                | Definition::Struct(Struct { name, .. })
                | Definition::Enum(Enum { name, .. })
                | Definition::Dict(Dict { name, .. })
                | Definition::Extern(name) => {
                    let ty = name.camel_case();
                    quote! {
                        ::nisba::const_try!(<#ty as ::nisba::decode::Decode>::decode(r))
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
