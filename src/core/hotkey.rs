//! 快捷键系统 —— 类似 Photoshop 的键位映射
//!
//! 设计原则：
//! 1. 文本输入框聚焦时，快捷键被抑制（由 iced `keyboard::listen()` 自动处理）
//! 2. 组合键（Ctrl/Alt/Shift）优先于单键
//! 3. 支持用户自定义覆盖
//! 4. 忽略输入法组合键（IME）与浏览器默认保留键

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// 修饰键
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[derive(Default)]
pub struct Modifiers {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
}


impl Modifiers {}

/// 快捷键绑定的动作标识
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum HotkeyAction {
    // ── 工具切换 ──
    ToolAuto,       // A
    ToolKeep,       // B
    ToolErase,      // E
    ToolPicker,     // I
    ToolPan,        // M / 空格
    ToolCrop,       // C
    // ── 视图 ──
    ZoomFit,        // 0
    ZoomIn,         // Ctrl + =
    ZoomOut,        // Ctrl + -
    ViewCompare,    // V
    ViewOriginal,   // O
    ViewResult,     // R
    // ── 编辑 ──
    Undo,           // Ctrl + Z
    ResetMask,      // Ctrl + R
    Export,         // Ctrl + S
    OpenImage,      // Ctrl + O
    RotateCW,       // Ctrl + ]
    RotateCCW,      // Ctrl + [
    // ── 画笔 ──
    BrushSizeUp,    // ]
    BrushSizeDown,  // [
    // ── 导航 ──
    ToggleNav,      // Ctrl + B
    // ── 裁剪 ──
    CropConfirm,    // Enter (裁剪模式)
    CropCancel,     // Esc (裁剪模式)
}

/// 默认快捷键映射（类似 Photoshop）
pub fn default_keymap() -> HashMap<HotkeyAction, (Modifiers, String)> {
    let mut map = HashMap::new();
    map.insert(HotkeyAction::ToolAuto,       (Modifiers::default(), "q".into()));
    map.insert(HotkeyAction::ToolKeep,       (Modifiers::default(), "b".into()));
    map.insert(HotkeyAction::ToolErase,      (Modifiers::default(), "e".into()));
    map.insert(HotkeyAction::ToolPicker,     (Modifiers::default(), "i".into()));
    map.insert(HotkeyAction::ToolPan,        (Modifiers::default(), "m".into()));
    map.insert(HotkeyAction::ToolCrop,       (Modifiers::default(), "c".into()));
    map.insert(HotkeyAction::ZoomFit,        (Modifiers::default(), "0".into()));
    map.insert(HotkeyAction::ZoomIn,         (Modifiers { ctrl: true, ..Default::default() }, "=".into()));
    map.insert(HotkeyAction::ZoomOut,        (Modifiers { ctrl: true, ..Default::default() }, "-".into()));
    map.insert(HotkeyAction::ViewCompare,    (Modifiers::default(), "s".into()));
    map.insert(HotkeyAction::ViewOriginal,   (Modifiers::default(), "a".into()));
    map.insert(HotkeyAction::ViewResult,     (Modifiers::default(), "d".into()));
    map.insert(HotkeyAction::Undo,           (Modifiers { ctrl: true, ..Default::default() }, "z".into()));
    map.insert(HotkeyAction::ResetMask,      (Modifiers { ctrl: true, ..Default::default() }, "r".into()));
    map.insert(HotkeyAction::Export,         (Modifiers { ctrl: true, ..Default::default() }, "s".into()));
    map.insert(HotkeyAction::OpenImage,      (Modifiers { ctrl: true, ..Default::default() }, "o".into()));
    map.insert(HotkeyAction::RotateCW,       (Modifiers { ctrl: true, ..Default::default() }, "]".into()));
    map.insert(HotkeyAction::RotateCCW,      (Modifiers { ctrl: true, ..Default::default() }, "[".into()));
    map.insert(HotkeyAction::BrushSizeUp,    (Modifiers::default(), "]".into()));
    map.insert(HotkeyAction::BrushSizeDown,  (Modifiers::default(), "[".into()));
    map.insert(HotkeyAction::ToggleNav,      (Modifiers { ctrl: true, ..Default::default() }, "b".into()));
    map
}

/// 用户自定义快捷键覆盖
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct KeymapOverrides {
    /// key = action 的 JSON 字符串表示，value = (modifiers, key_char)
    pub overrides: HashMap<String, (Modifiers, String)>,
}

impl KeymapOverrides {
    /// 合并默认映射与用户覆盖
    pub fn resolve(&self) -> HashMap<HotkeyAction, (Modifiers, String)> {
        let mut map = default_keymap();
        for (action_str, binding) in &self.overrides {
            if let Ok(action) = serde_json::from_str::<HotkeyAction>(action_str) {
                map.insert(action, binding.clone());
            }
        }
        map
    }
}

/// 快捷键描述（用于 UI 展示）
pub fn action_label(action: &HotkeyAction) -> &'static str {
    match action {
        HotkeyAction::ToolAuto => "智能抠图",
        HotkeyAction::ToolKeep => "保留画笔",
        HotkeyAction::ToolErase => "擦除画笔",
        HotkeyAction::ToolPicker => "吸管",
        HotkeyAction::ToolPan => "移动画布",
        HotkeyAction::ToolCrop => "裁剪",
        HotkeyAction::ZoomFit => "适应窗口",
        HotkeyAction::ZoomIn => "放大",
        HotkeyAction::ZoomOut => "缩小",
        HotkeyAction::ViewCompare => "对比视图",
        HotkeyAction::ViewOriginal => "原图视图",
        HotkeyAction::ViewResult => "结果视图",
        HotkeyAction::Undo => "撤销",
        HotkeyAction::ResetMask => "重置蒙版",
        HotkeyAction::Export => "导出",
        HotkeyAction::OpenImage => "打开图片",
        HotkeyAction::RotateCW => "顺时针旋转",
        HotkeyAction::RotateCCW => "逆时针旋转",
        HotkeyAction::BrushSizeUp => "笔刷变大",
        HotkeyAction::BrushSizeDown => "笔刷变小",
        HotkeyAction::ToggleNav => "折叠导航",
        HotkeyAction::CropConfirm => "确认裁剪",
        HotkeyAction::CropCancel => "取消裁剪",
    }
}

/// 格式化修饰键显示（如 "Ctrl+Shift+A"）
pub fn format_binding(modifiers: &Modifiers, key: &str) -> String {
    let mut parts = Vec::new();
    if modifiers.ctrl {
        parts.push("Ctrl");
    }
    if modifiers.shift {
        parts.push("Shift");
    }
    if modifiers.alt {
        parts.push("Alt");
    }
    parts.push(key);
    parts.join("+")
}