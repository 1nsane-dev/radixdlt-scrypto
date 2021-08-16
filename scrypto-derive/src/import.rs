use std::fs;

use proc_macro2::{Ident, Span, TokenStream};
use quote::{format_ident, quote};
use syn::punctuated::Punctuated;
use syn::token::Comma;
use syn::*;

macro_rules! trace {
    ($($arg:expr),*) => {{
        #[cfg(feature = "trace")]
        println!($($arg),*);
    }};
}

pub fn handle_import(input: TokenStream) -> TokenStream {
    trace!("handle_import() starts");
    let span = Span::call_site();

    let path_lit: LitStr = parse2(input).expect("Unable to parse input");
    let path = path_lit.value();
    let abi = fs::read_to_string(path).expect("Unable to load Abi");
    let component: scrypto_abi::Component =
        serde_json::from_str(abi.as_str()).expect("Unable to parse Abi");
    trace!("ABI: {:?}", component);

    let mut items: Vec<Item> = vec![];
    let mut implementations: Vec<ItemImpl> = vec![];

    let blueprint_address = component.blueprint;
    let component_name = component.name;
    let ident = Ident::new(component_name.as_str(), span);
    trace!("Component name: {}", quote! { #ident });

    let structure: Item = parse_quote! {
        pub struct #ident {
            address: scrypto::types::Address
        }
    };
    items.push(structure);

    let mut functions = Vec::<ItemFn>::new();
    functions.push(parse_quote! {
        pub fn from_address(address: scrypto::types::Address) -> Self {
            Self {
                address
            }
        }
    });

    for method in &component.methods {
        trace!("Processing method: {:?}", method);

        let method_name = &method.name;
        let func_indent = Ident::new(method_name.as_str(), span);
        let mut func_inputs = Punctuated::<FnArg, Comma>::new();
        let mut func_args = Vec::<Ident>::new();

        let address_stmt: syn::Stmt;
        match method.mutability {
            scrypto_abi::Mutability::Immutable => {
                func_inputs.push(parse_quote! { &self });
                address_stmt = parse_quote! { let address = self.address; };
            }
            scrypto_abi::Mutability::Mutable => {
                func_inputs.push(parse_quote! { &mut self });
                address_stmt = parse_quote! { let address = self.address; };
            }
            scrypto_abi::Mutability::Stateless => {
                address_stmt = parse_quote! { let address = scrypto::types::Address::from_hex(#blueprint_address).unwrap(); };
            }
        }

        for (i, input) in method.inputs.iter().enumerate() {
            match input {
                _ => {
                    let ident = format_ident!("arg{}", i);
                    let (new_type, new_items) = get_native_type(input);
                    func_args.push(parse_quote! { #ident });
                    func_inputs.push(parse_quote! { #ident: #new_type });
                    items.extend(new_items);
                }
            }
            if i < method.inputs.len() - 1 {
                func_inputs.push_punct(Comma(span));
            }
        }
        let (func_output, new_items) = get_native_type(&method.output);
        items.extend(new_items);

        functions.push(parse_quote! {
            pub fn #func_indent(#func_inputs) -> #func_output {
                #address_stmt
                scrypto::call!(#func_output, #component_name, #method_name, address #(, #func_args)*)
            }
        });
    }

    let implementation = parse_quote! {
        impl #ident {
            #(#functions)*
        }
    };
    trace!("Implementation: {}", quote! { #implementation });
    implementations.push(implementation);

    let output = quote! {
         #(#items)*

         #(#implementations)*
    };
    trace!("handle_import() finishes");

    #[cfg(feature = "trace")]
    crate::utils::print_compiled_code("import!", &output);

    output.into()
}

fn get_native_type(ty: &sbor::types::Type) -> (Type, Vec<Item>) {
    let mut items = Vec::<Item>::new();

    let t: Type = match ty {
        sbor::types::Type::Unit => parse_quote! { () },
        sbor::types::Type::Bool => parse_quote! { bool },
        sbor::types::Type::I8 => parse_quote! { i8 },
        sbor::types::Type::I16 => parse_quote! { i16 },
        sbor::types::Type::I32 => parse_quote! { i32 },
        sbor::types::Type::I64 => parse_quote! { i64 },
        sbor::types::Type::I128 => parse_quote! { i128 },
        sbor::types::Type::U8 => parse_quote! { u8 },
        sbor::types::Type::U16 => parse_quote! { u16 },
        sbor::types::Type::U32 => parse_quote! { u32 },
        sbor::types::Type::U64 => parse_quote! { u64 },
        sbor::types::Type::U128 => parse_quote! { u128 },
        sbor::types::Type::String => parse_quote! { String },
        sbor::types::Type::H256 => parse_quote! { scrypto::types::H256 },
        sbor::types::Type::U256 => parse_quote! { scrypto::types::U256 },
        sbor::types::Type::Address => parse_quote! { scrypto::types::Address },
        sbor::types::Type::BID => parse_quote! { scrypto::types::BID },
        sbor::types::Type::RID => parse_quote! { scrypto::types::RID },
        sbor::types::Type::Option { value } => {
            let (new_type, new_items) = get_native_type(value);
            items.extend(new_items);

            parse_quote! { Option<#new_type> }
        }
        sbor::types::Type::Box { value } => {
            let (new_type, new_items) = get_native_type(value);
            items.extend(new_items);

            parse_quote! { Box<#new_type> }
        }
        sbor::types::Type::Struct { name, fields } => {
            let ident = format_ident!("{}", name);

            match fields {
                sbor::types::Fields::Named { named } => {
                    let names: Vec<Ident> =
                        named.iter().map(|k| format_ident!("{}", k.0)).collect();
                    let mut types: Vec<Type> = vec![];
                    for (_, v) in named {
                        let (new_type, new_items) = get_native_type(v);
                        types.push(new_type);
                        items.extend(new_items);
                    }
                    items.push(parse_quote! {
                        #[derive(Debug, sbor::Encode, sbor::Decode)]
                        pub struct #ident {
                            #( pub #names : #types, )*
                        }
                    });
                }
                _ => {
                    todo!("Add support for non-named fields")
                }
            }

            parse_quote! { #ident }
        }
        sbor::types::Type::Tuple { elements } => {
            let mut types: Vec<Type> = vec![];

            for element in elements {
                let (new_type, new_items) = get_native_type(element);
                types.push(new_type);
                items.extend(new_items);
            }

            parse_quote! { ( #(#types),* ) }
        }
        sbor::types::Type::Array { element, length } => {
            let (new_type, new_items) = get_native_type(element);
            items.extend(new_items);

            let n = *length as usize;
            parse_quote! { [#new_type; #n] }
        }
        sbor::types::Type::Enum { name, variants } => {
            let ident = format_ident!("{}", name);
            let mut native_variants = Vec::<Variant>::new();

            for variant in variants {
                let v_ident = format_ident!("{}", variant.name);

                match &variant.fields {
                    sbor::types::Fields::Named { named } => {
                        let mut names: Vec<Ident> = vec![];
                        let mut types: Vec<Type> = vec![];
                        for (n, v) in named {
                            names.push(format_ident!("{}", n));
                            let (new_type, new_items) = get_native_type(&v);
                            types.push(new_type);
                            items.extend(new_items);
                        }
                        native_variants.push(parse_quote! {
                            #v_ident {
                                #(#names: #types),*
                            }
                        });
                    }
                    sbor::types::Fields::Unnamed { unnamed } => {
                        let mut types: Vec<Type> = vec![];
                        for v in unnamed {
                            let (new_type, new_items) = get_native_type(&v);
                            types.push(new_type);
                            items.extend(new_items);
                        }
                        native_variants.push(parse_quote! {
                            #v_ident ( #(#types),* )
                        });
                    }
                    sbor::types::Fields::Unit => {
                        native_variants.push(parse_quote! {
                            #v_ident
                        });
                    }
                };
            }

            items.push(parse_quote! {
                #[derive(Debug, sbor::Encode, sbor::Decode)]
                pub enum #ident {
                    #( #native_variants ),*
                }
            });

            parse_quote! { #ident }
        }
        sbor::types::Type::Vec { element } => {
            let (new_type, new_items) = get_native_type(element);
            items.extend(new_items);

            parse_quote! { Vec<#new_type> }
        }
        sbor::types::Type::TreeSet { element } => {
            let (new_type, new_items) = get_native_type(element);
            items.extend(new_items);

            parse_quote! { BTreeSet<#new_type> }
        }
        sbor::types::Type::TreeMap { key, value } => {
            let (key_type, new_items) = get_native_type(key);
            items.extend(new_items);
            let (value_type, new_items) = get_native_type(value);
            items.extend(new_items);

            parse_quote! { BTreeMap<#key_type, #value_type> }
        }
        sbor::types::Type::HashSet { element } => {
            let (new_type, new_items) = get_native_type(element);
            items.extend(new_items);

            parse_quote! { HashSet<#new_type> }
        }
        sbor::types::Type::HashMap { key, value } => {
            let (key_type, new_items) = get_native_type(key);
            items.extend(new_items);
            let (value_type, new_items) = get_native_type(value);
            items.extend(new_items);

            parse_quote! { HashMap<#key_type, #value_type> }
        }
    };

    (t, items)
}

#[cfg(test)]
mod tests {
    use proc_macro2::TokenStream;
    use std::str::FromStr;

    use super::*;

    fn assert_code_eq(a: TokenStream, b: TokenStream) {
        assert_eq!(a.to_string(), b.to_string());
    }

    #[test]
    fn test_import() {
        let input = TokenStream::from_str("\"../scrypto-tests/tests/abi.json\"").unwrap();
        let output = handle_import(input);

        assert_code_eq(
            output,
            quote! {
                pub struct Sample {
                    address: scrypto::types::Address
                }
                #[derive(Debug, sbor::Encode, sbor::Decode)]
                pub struct Floor {
                    pub x: u32,
                    pub y: u32,
                }
                #[derive(Debug, sbor::Encode, sbor::Decode)]
                pub enum Hello {
                    A { x: u32 },
                    B(u32),
                    C
                }
                impl Sample {
                    pub fn from_address(address: scrypto::types::Address) -> Self {
                        Self { address }
                    }
                    pub fn stateless_func() -> u32 {
                        let address = scrypto::types::Address::from_hex(
                            "056967d3d49213394892980af59be76e9b3e7cc4cb78237460d0c7"
                        )
                        .unwrap();
                        scrypto::call!(u32, "Sample", "stateless_func", address)
                    }
                    pub fn calculate_volume(
                        &self,
                        arg0: Floor,
                        arg1: (u8, u16),
                        arg2: Vec<String>,
                        arg3: u32,
                        arg4: Hello,
                        arg5: [String; 2usize]
                    ) -> u32 {
                        let address = self.address;
                        scrypto::call!(
                            u32,
                            "Sample",
                            "calculate_volume",
                            address,
                            arg0,
                            arg1,
                            arg2,
                            arg3,
                            arg4,
                            arg5
                        )
                    }
                }
            },
        );
    }
}
