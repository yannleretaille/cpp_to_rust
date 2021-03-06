// #![forbid(unused_must_use)]

#![cfg_attr(feature="clippy", feature(plugin))]
#![cfg_attr(feature="clippy", plugin(clippy))]
#![cfg_attr(feature="clippy", warn(nonminimal_bool))]
#![cfg_attr(feature="clippy", warn(if_not_else))]
#![cfg_attr(feature="clippy", warn(shadow_same))]
#![cfg_attr(feature="clippy", warn(shadow_unrelated))]
#![cfg_attr(feature="clippy", warn(single_match_else))]
// some time in the future...
// #![warn(option_unwrap_used)]
// #![warn(result_unwrap_used)]
// #![warn(print_stdout)]

#![recursion_limit = "1024"] // for error_chain
#[macro_use]
extern crate error_chain;


mod cpp_ffi_generator;
mod cpp_code_generator;
mod caption_strategy;
pub mod config;
pub mod cpp_data;
mod cpp_ffi_data;
mod cpp_lib_builder;
pub mod cpp_method;
pub mod cpp_type;
pub mod cpp_operator;
mod dependency_info;
mod doc_formatter;
pub mod errors;
pub mod file_utils;
mod launcher;
pub mod log;
mod qt_doc_parser;
mod rust_generator;
mod rust_code_generator;
mod rust_info;
mod rust_type;
pub mod utils;
mod cpp_parser;
mod serializable;
pub mod string_utils;

#[cfg(test)]
mod tests;
