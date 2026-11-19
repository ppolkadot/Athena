pub mod appearance;
pub mod components;
pub mod icons;
#[rustfmt::skip]
mod icon_paths;
pub mod quick_add;
pub mod quick_add_prompts;
pub mod screens;
pub mod shell;
pub mod theme;
pub mod tokens;

#[cfg(test)]
mod appearance_snapshot_tests;
#[cfg(test)]
mod quick_add_flow_tests;
#[cfg(test)]
pub(crate) mod snapshot_tests;
