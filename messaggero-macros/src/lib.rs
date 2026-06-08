use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, DeriveInput};

/// Derive macro that generates an `AgentCard` builder from struct-level attributes.
///
/// # Example
///
/// ```ignore
/// #[derive(AgentCard)]
/// #[agent(name = "echo", description = "Echoes messages")]
/// struct EchoAgent;
/// ```
#[proc_macro_derive(AgentCard, attributes(agent))]
pub fn derive_agent_card(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = &input.ident;

    let mut agent_name = String::new();
    let mut agent_desc = String::new();

    for attr in &input.attrs {
        if attr.path().is_ident("agent") {
            let _ = attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("name") {
                    let value = meta.value()?;
                    let lit: syn::LitStr = value.parse()?;
                    agent_name = lit.value();
                } else if meta.path.is_ident("description") {
                    let value = meta.value()?;
                    let lit: syn::LitStr = value.parse()?;
                    agent_desc = lit.value();
                }
                Ok(())
            });
        }
    }

    if agent_name.is_empty() {
        agent_name = name.to_string().to_lowercase();
    }

    let expanded = quote! {
        impl #name {
            pub fn agent_card() -> messaggero_core::AgentCard {
                messaggero_core::AgentCard::builder(#agent_name)
                    .description(#agent_desc)
                    .build()
            }
        }
    };

    TokenStream::from(expanded)
}
