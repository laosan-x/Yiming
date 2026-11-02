//! 撤销栈：为破坏性编辑（旋转落盘 / 裁剪确认）提供回退
//!
//! 与原项目 `core/history.js` 同策略：
//!   1) 快照存值而非引用（Rust 无法共享可变），但只在旋转 / 裁剪时打点，
//!      笔刷改动发生在快照之后，不受影响；
//!   2) 深度上限 3 —— 一份快照约 6 字节/像素，2048² 约 25MB，3 份封顶；
//!   3) 换图即清空。

use image::RgbaImage;

const MAX: usize = 3;

#[derive(Debug, Clone)]
pub struct Snapshot {
    pub src: std::sync::Arc<RgbaImage>,
    pub mask: Vec<u8>,
    pub mask_base: Vec<u8>,
    pub rot_total: f32,
}

#[derive(Default)]
pub struct History {
    stack: Vec<Snapshot>,
}

impl History {
    pub fn push(&mut self, snap: Snapshot) {
        self.stack.push(snap);
        if self.stack.len() > MAX {
            self.stack.remove(0);
        }
    }
    pub fn pop(&mut self) -> Option<Snapshot> {
        self.stack.pop()
    }
    pub fn can_undo(&self) -> bool {
        !self.stack.is_empty()
    }
    pub fn clear(&mut self) {
        self.stack.clear();
    }
    pub fn depth(&self) -> usize {
        self.stack.len()
    }
}
