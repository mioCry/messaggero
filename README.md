# messaggero

[![CI](https://github.com/mioCry/messaggero/actions/workflows/ci.yml/badge.svg)](https://github.com/mioCry/messaggero/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/messaggero.svg)](https://crates.io/crates/messaggero)
[![docs.rs](https://docs.rs/messaggero/badge.svg)](https://docs.rs/messaggero)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](LICENSE-MIT)
[![MSRV: 1.78](https://img.shields.io/badge/msrv-1.78-orange.svg)](https://blog.rust-lang.org/2024/05/02/Rust-1.78.0.html)

A Rust library for building high-performance multi-agent systems with a protocol
designed around two complementary transport modes: a binary fast path for
agents running on the same host, and an A2A-compatible HTTP transport for
cross-vendor interoperability.
