//! `legacy-modernizer` — a standalone, air-gap-friendly implementation of
//! the six-agent Legacy Modernization Orchestrator: Discovery & Inventory,
//! Risk & Triage, Architecture & Decoupling, Code Generation, Verification
//! & Test, and Documentation & Audit.
//!
//! This crate is a library plus a thin CLI binary (`src/main.rs`) so that
//! the pipeline is directly testable without shelling out to itself —
//! see `demo::run_full_demo` and `tests/prototype_demo.rs`.

pub mod architecture;
pub mod codeblocks;
pub mod codegen;
pub mod config;
pub mod demo;
pub mod discovery;
pub mod document;
pub mod fsscan;
pub mod llm;
pub mod prompts;
pub mod risk_triage;
pub mod state;
pub mod verify;
