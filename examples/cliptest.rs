//! 剪贴板互通的验证用小程序（examples/ 不会被 cargo build 编进正式产物）
//!
//!   cargo run --example cliptest -- write <path> [cut]
//!   cargo run --example cliptest -- read
//!   cargo run --example cliptest -- has
//!
//! 用法：cliptest write 之后用别的程序（Python ctypes / Explorer）验证能否粘贴；
//! 反过来用别的程序写剪贴板，cliptest read 验证能否读出来。

#[path = "../src/fs_util.rs"]
mod fs_util;

use std::path::PathBuf;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(|s| s.as_str()) {
        Some("write") => {
            let paths: Vec<PathBuf> = args[1..]
                .iter()
                .filter(|a| *a != "cut")
                .map(PathBuf::from)
                .collect();
            let cut = args.iter().any(|a| a == "cut");
            match fs_util::clip_write_files(&paths, cut) {
                Ok(()) => println!("OK wrote {} item(s) cut={}", paths.len(), cut),
                Err(e) => println!("ERR {}", e),
            }
        }
        Some("read") => match fs_util::clip_read_files() {
            Some((p, cut)) => {
                println!("count={} cut={}", p.len(), cut);
                for x in p {
                    println!("{}", x.display());
                }
            }
            None => println!("EMPTY"),
        },
        Some("has") => println!("has_files={}", fs_util::clip_has_files()),
        Some("clear") => {
            fs_util::clip_clear();
            println!("OK cleared");
        }
        _ => println!("usage: cliptest write <path...> [cut] | read | has | clear"),
    }
}
