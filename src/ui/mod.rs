//! Screens, dialogs and widgets.

pub mod cityfounded;
pub mod city_draw;
pub mod city_view;
pub mod conquest;
pub mod cityscreen;
pub mod civilopedia;
pub mod context;
pub mod dialogs;
pub mod diplomacy_ui;
pub mod discovery;
pub mod endgame;
pub mod eventpics;
pub mod govtscreen;
pub mod halloffame;
pub mod loadgame;
pub mod intro_message;
pub mod mapview;
pub mod palace;
pub mod pedia_data;
pub mod pedia_entry;
pub mod report_advisors;
pub mod report_world;
pub mod replay;
pub mod reports;
pub mod reveal;
pub mod screen;
pub mod setup;
pub mod spaceship_ui;
pub mod story;
pub mod theme;
pub mod tilerender;
pub mod title;
pub mod unit_actions;
pub mod widgets;
pub mod wonderscreen;

pub use screen::{DialogResult, Screen, ScreenStack, Transition};
