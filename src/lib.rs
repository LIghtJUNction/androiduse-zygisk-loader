mod api;
mod binding;
#[doc(hidden)]
pub mod macros;
mod module;

#[macro_use]
extern crate log;
#[cfg(target_os = "android")]
extern crate android_logger;

#[cfg(target_os = "android")]
use android_logger::Config;
#[cfg(target_os = "android")]
use log::LevelFilter;

use std::ffi::{CStr, CString};
use std::fs::File;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::sync::OnceLock;

pub use api::ZygiskApi;
pub use binding::{AppSpecializeArgs, ServerSpecializeArgs, StateFlags, ZygiskOption, API_VERSION};
use jni::{JNIEnv, JavaVM};
pub use module::ZygiskModule;

const CONFIG_PATH: &str = "/data/adb/modules/AndroidUse/.config/androiduse/zygisk-target";
const SOURCE_PAYLOAD_PATH: &str = "/data/adb/modules/AndroidUse/.config/androiduse/payload.so";

static MODULE: ZygiskLoaderModule = ZygiskLoaderModule {};
crate::zygisk_module!(&MODULE);

struct ZygiskLoaderModule {}

static JAVA_VM: OnceLock<JavaVM> = OnceLock::new();
static TARGET_CONFIG: OnceLock<String> = OnceLock::new();
static PAYLOAD_BUFFER: OnceLock<Vec<u8>> = OnceLock::new();
static TARGET_APP_DETECTED: OnceLock<bool> = OnceLock::new();

fn rand_int() -> u32 {
    // Simple pseudo-random for filename obfuscation using time
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .subsec_nanos()
}

fn write_file(path: &str, data: &[u8]) -> std::io::Result<()> {
    let mut f = File::create(path)?;
    f.write_all(data)?;
    f.sync_all()?;
    let permissions = std::fs::Permissions::from_mode(0o700);
    std::fs::set_permissions(path, permissions)?;
    Ok(())
}

fn read_file_to_memory(path: &str) -> std::io::Result<Vec<u8>> {
    let mut f = File::open(path)?;
    let mut buffer = Vec::new();
    f.read_to_end(&mut buffer)?;
    Ok(buffer)
}

impl ZygiskModule for ZygiskLoaderModule {
    fn on_load(&self, _api: ZygiskApi, env: &mut JNIEnv) {
        #[cfg(target_os = "android")]
        android_logger::init_once(
            Config::default()
                .with_max_level(LevelFilter::Debug)
                .with_tag("AndroidUseZygisk"),
        );

        let vm = env.get_java_vm().expect("Failed to get JavaVM");
        let _ = JAVA_VM.set(vm);
        info!("AndroidUse Zygisk loader initialized");
    }

    fn pre_app_specialize(&self, _api: ZygiskApi, args: &mut AppSpecializeArgs) {
        // 1. Read Config (As Root/Zygote)
        if let Ok(target) = read_target_config() {
            let _ = TARGET_CONFIG.set(target);
        }

        let current_process = get_process_name_from_args_safe(args);
        let target_package = TARGET_CONFIG.get().map(|s| s.as_str()).unwrap_or("");

        if !target_package.is_empty() && current_process.contains(target_package) {
            info!("Target Detected: {}", current_process);
            let _ = TARGET_APP_DETECTED.set(true);

            // 2. Read Payload to RAM
            match read_file_to_memory(SOURCE_PAYLOAD_PATH) {
                Ok(buffer) => {
                    info!("Payload buffered to RAM: {} bytes", buffer.len());
                    let _ = PAYLOAD_BUFFER.set(buffer);
                }
                Err(e) => {
                    error!(
                        "Failed to buffer payload from {}: {}",
                        SOURCE_PAYLOAD_PATH, e
                    );
                }
            }
        }
    }

    fn post_app_specialize(&self, _api: ZygiskApi, args: &AppSpecializeArgs) {
        if TARGET_APP_DETECTED.get() != Some(&true) {
            return;
        }

        if let Some(buffer) = PAYLOAD_BUFFER.get() {
            // FIX: Use app_data_dir directly instead of nice_name
            // This ensures we write to the correct folder even for isolated processes (e.g., :remote)
            let data_dir = get_app_data_dir_from_args(args);

            if data_dir.is_empty() {
                error!("Could not determine app data directory");
                return;
            }

            // Generate a random filename to avoid collisions and look like a cache file
            let file_name = format!("{}/cache/.res_{}.so", data_dir, rand_int());

            info!("Attempting injection to: {}", file_name);

            match write_file(&file_name, buffer) {
                Ok(_) => {
                    let c_path = CString::new(file_name.clone()).unwrap();
                    unsafe {
                        let handle = libc::dlopen(c_path.as_ptr(), libc::RTLD_NOW);

                        // Immediately unlink (delete) the file from disk
                        // The kernel keeps the file in memory as long as it's mapped,
                        // but the file entry is removed from the filesystem.
                        let _ = std::fs::remove_file(&file_name);

                        if handle.is_null() {
                            let err = CStr::from_ptr(libc::dlerror()).to_string_lossy();
                            error!("Injection failed: {}", err);
                        } else {
                            info!("Injection success! Handle: {:p}", handle);
                        }
                    }
                }
                Err(e) => error!("Failed to write payload: {}", e),
            }
        }
    }
}

fn read_target_config() -> std::io::Result<String> {
    let f = File::open(CONFIG_PATH)?;
    let mut reader = BufReader::new(f);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    Ok(line.trim().to_string())
}

// ARGS PARSING HELPERS

fn get_process_name_from_args_safe(args: &AppSpecializeArgs) -> String {
    if let Some(vm) = JAVA_VM.get() {
        // Fast-Path: Thread already attached in Zygote child process
        if let Ok(mut env) = vm.get_env() {
            if let Ok(s) = env.get_string(args.nice_name) {
                let s_rust: String = s.into();
                if !s_rust.is_empty() {
                    return s_rust;
                }
            }
        }
    }
    let dir = get_app_data_dir_from_args(args);
    if !dir.is_empty() {
        return extract_package_from_path(&dir);
    }
    String::new()
}

fn get_app_data_dir_from_args(args: &AppSpecializeArgs) -> String {
    if let Some(vm) = JAVA_VM.get() {
        // Fast-Path: Thread already attached in Zygote child process
        if let Ok(mut env) = vm.get_env() {
            if let Ok(j_str) = env.get_string(args.app_data_dir) {
                return j_str.into();
            }
        }
    }
    String::new()
}

fn extract_package_from_path(path: &str) -> String {
    let parts: Vec<&str> = path.split('/').collect();
    if parts.len() >= 3 {
        for part in parts.iter().rev() {
            if !part.is_empty() && *part != "cache" {
                return part.to_string();
            }
        }
    }
    String::new()
}

#[cfg(test)]
mod test {
    use std::os::unix::io::RawFd;
    fn companion(_socket: RawFd) {}
    crate::zygisk_companion!(companion);
}
