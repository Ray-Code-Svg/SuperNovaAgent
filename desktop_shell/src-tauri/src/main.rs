#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    if product_runtime::run_kernel_worker_from_args_if_requested() {
        return;
    }
    supernova_desktop_shell_lib::run()
}
