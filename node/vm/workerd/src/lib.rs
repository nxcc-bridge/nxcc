#![allow(warnings)]

pub mod config;
pub mod config_builder;
pub mod errors;
pub mod vmm;

#[allow(warnings)]
pub mod workerd_capnp {
    include!(concat!(env!("OUT_DIR"), "/workerd_capnp.rs"));
}
