// 发布版本不弹出额外的控制台窗口，不要删除！
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    disklens_lib::run()
}
