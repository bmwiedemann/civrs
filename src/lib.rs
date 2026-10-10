//! civrs: a Rust reimplementation of MicroProse Civilization (DOS, 1991) that reads the
//! original game assets at runtime and renders the 320x200 indexed-colour display through SDL3.

pub mod ai;
pub mod app;
pub mod assets;
pub mod audio;
pub mod gfx;
pub mod platform;
pub mod game;
pub mod rules;
pub mod save;
pub mod ui;
