//! 与 UI 无关的内核：抠图算法、证件照规格与排版、图像 IO、配置持久化、撤销栈。

pub mod config;
pub mod history;
pub mod i18n;
pub mod hotkey;
pub mod idphoto;
pub mod image;
pub mod matting;
pub mod nn;
pub mod notify;
pub mod pyramid;
pub mod updater;