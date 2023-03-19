//! This build script copies the `memory.x` file from the crate root into
//! a directory where the linker can always find it at build time.
//! For many projects this is optional, as the linker always searches the
//! project root directory -- wherever `Cargo.toml` is. However, if you
//! are using a workspace or have a more complicated build setup, this
//! build script becomes required. Additionally, by requesting that
//! Cargo re-run the build script whenever `memory.x` is changed,
//! updating `memory.x` ensures a rebuild of the application with the
//! new memory settings.

use std::env;
use std::fs::File;
use std::io::Write;
use std::path::PathBuf;

use dotenvy::dotenv;

fn main() {
    dotenv().ok();
    let wifi_network = env::var("WIFI_NETWORK").unwrap();
    println!("cargo:rustc-env=WIFI_NETWORK={wifi_network}");
    let wifi_password = env::var("WIFI_PASSWORD").unwrap();
    println!("cargo:rustc-env=WIFI_PASSWORD={wifi_password}");
    let discord_bot_token = env::var("DISCORD_BOT_TOKEN").unwrap();
    println!("cargo:rustc-env=DISCORD_BOT_TOKEN={discord_bot_token}");
    let discord_channel_id = env::var("DISCORD_CHANNEL_ID").unwrap();
    println!("cargo:rustc-env=DISCORD_CHANNEL_ID={discord_channel_id}");

    // Put `memory.x` in our output directory and ensure it's
    // on the linker search path.
    let out = &PathBuf::from(env::var_os("OUT_DIR").unwrap());
    File::create(out.join("memory.x"))
        .unwrap()
        .write_all(include_bytes!("memory.x"))
        .unwrap();
    println!("cargo:rustc-link-search={}", out.display());

    // By default, Cargo will re-run a build script whenever
    // any file in the project changes. By specifying `memory.x`
    // here, we ensure the build script is only re-run when
    // `memory.x` is changed.
    println!("cargo:rerun-if-changed=memory.x");

    println!("cargo:rustc-link-arg-bins=--nmagic");
    println!("cargo:rustc-link-arg-bins=-Tlink.x");
    println!("cargo:rustc-link-arg-bins=-Tlink-rp.x");
    println!("cargo:rustc-link-arg-bins=-Tdefmt.x");
}
